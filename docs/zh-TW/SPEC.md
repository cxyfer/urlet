[English](../SPEC.md) | **繁體中文**

# urlet — 輕量化多用戶短網址系統規格書 (Spec)

> 版本：1.8
> 狀態：Approved for implementation
> 最後更新：2026-10-28

**目錄**

1. 概述 (Overview)
2. 名詞定義
3. 系統架構
4. 功能需求
5. 非功能需求
6. 資料模型
7. API 規格
8. 設定（環境變數）
9. 部署
10. 測試策略
11. 驗收標準 (Definition of Done)
12. 未來擴充路徑

---

## 1. 概述 (Overview)

`urlet` 是一套**自託管、多用戶**的短網址服務。每位使用者擁有自己獨立的短網址空間，
可建立短網址、設定**密碼保護**與**過期機制**、檢視點擊統計，並透過個人 API Token 以程式方式操作。

系統以「**盡可能輕量**」為最高設計原則：單一 Rust 二進位檔 + 單一 SQLite 檔案，
一個 `docker compose up -d` 即可完成部署，不需要外部資料庫、快取或前端建置流程。
v1.1 起介面提供三種語言（英文／繁體中文／簡體中文）、可選的防機器人驗證，並新增公開導覽首頁。
v1.2 起新增**使用者群組**：可為不同群組設定各自的連結與 API Token 配額及功能開關，
讓同一套單一二進位部署能支援分級方案（例如免費組與付費組）。
v1.4 起新增**可切換主題**：視覺識別可由編譯進 binary 的模板集與樣式表組成，管理員也可上傳純 CSS 的自訂主題；管理員決定提供哪些設計與誰能切換，使用者則以 cookie 記住選擇。
v1.5 起新增**中轉頁 (Splash Page)**：連結可先顯示帶倒數的宣傳頁，再由分組政策決定成員只能用直接跳轉、只能中轉頁，或可自行選擇。
v1.6 起新增**公開短網址工具**：導覽首頁可提供免帳號訪客使用的縮網址表單，並由管理員控制存取層級、
逐欄位開關、防機器人驗證，以及每來源位址與全體訪客共用的速率限制。

### 1.1 核心設計原則 (來自需求方)

| # | 原則 | 落實方式 |
|---|------|----------|
| P1 | 多用戶，各自獨立管理自己的網址 | 每個資源查詢都強制帶入 `user_id` 所有權過濾；跨用戶存取一律 404 |
| P2 | 密碼保護 | 短網址層級 Argon2id 密碼雜湊 + 簽章解鎖 Cookie（TTL 30 分鐘） |
| P3 | 過期機制 | 絕對時間 `expires_at` + 點擊次數上限 `max_clicks`，過期後回 410 Gone |
| P4 | Docker Compose 部署 | 單一 `docker-compose.yml`（app + named volume），可選 Caddy TLS overlay |
| P5 | 盡可能輕量化 | 單一二進位、SQLite（無 MySQL/Redis）、SSR 模板（無 Node 建置）、映像檔 < 40 MB |
| P6 | Rust 實作 | Rust 2021 + Axum 0.8 + Tokio + sqlx(SQLite) |
| P7 | 完善的資料表設計 | 21 張表、37 個索引、軟刪除、稽核日誌、時間序列彙總表，便於未來擴充 |
| P8 | 適度添加功能 | 標籤、點擊分析、API Token、邀請碼、管理後台、稽核日誌、每日彙總、三語介面、可選防機器人驗證、配額／功能開關使用者群組、可切換與可上傳主題、中轉頁、供訪客使用的公開短網址工具 |

### 1.2 與先前半成品 (OURLS / OURLS2) 的關係

先前專案已探索過兩個方向，本專案**繼承其資料模型與功能構想**，但依 P5 重新選型：

| 項目 | OURLS（半成品） | OURLS2（半成品） | **urlet（本專案）** |
|------|----------------|------------------|----------------------|
| 執行環境 | Axum + MySQL + Redis | Cloudflare Workers (WASM) + D1 | **Axum + SQLite（單二進位）** |
| 部署 | docker compose（3 容器） | Wrangler / Cloudflare | **docker compose（1 容器）** |
| 認證 | JWT + Refresh Token + OAuth | JWT + OAuth | **伺服器端 Session + API Token** |
| 前端 | React + Vite + Tailwind（需 Node 建置） | React + Vite + Pages | **Askama SSR + 原生 CSS/JS（零建置）** |
| 權限 | 使用者群組 + 權限矩陣 | 群組 + 權限矩陣 | **角色 (user/admin) + 配額／功能開關群組**（無權限矩陣） |
| 統計 | 完整（GeoIP、MaxMind） | 完整 | **精簡（UA 解析、每日彙總、無 GeoIP）** |
| 繼承 | 密碼保護、過期、點擊上限、標籤、API Token、邀請碼、稽核日誌 | 同上 + 公開預覽 API | 上述全部，去蕪存菁 |

**刻意不做（Non-Goals）**：OAuth 第三方登入、細緻權限矩陣、GeoIP/MMDB、
關鍵字黑名單、A/B 測試、Webhook、QR Code、Link-in-bio、郵件寄送。

多語系在 v1.0 原列為 Non-Goal，自 v1.1 起納入範圍（見 §4.11）。使用者群組在 v1.0 原列為
Non-Goal，v1.2 起納入配額與功能開關（見 §4.13），但仍不做逐動作的權限矩陣。

---

## 2. 名詞定義

| 名詞 | 說明 |
|------|------|
| Slug | 短網址的路徑片段，例如 `https://s.example.com/`**`aB3xK9z`** |
| Target URL | 短網址導向的原始網址 |
| 解鎖 (Unlock) | 輸入正確的短網址密碼後，取得短期授權 Cookie 的流程 |
| 過期 (Expired) | `expires_at` 已過 **或** `max_clicks` 已達，回應 410 Gone |
| Session | 伺服器端登入狀態（`sessions` 表 + HttpOnly Cookie） |
| API Token | 供程式存取的長期憑證（`Authorization: Bearer <token>`） |
| 語言標籤 (Language tag) | 支援的介面語言代碼：`en`、`zh-Hant`、`zh-Hans` |
| 語言目錄 (Locale catalog) | 內嵌於二進位檔的 JSON 檔（`src/i18n/<tag>.json`），存放單一語言的訊息鍵值 |
| Captcha token | 由供應商簽發、隨表單送出並由伺服器端驗證的回應權杖 |
| 使用者群組 (User group) | 由管理員定義的配額與功能開關集合（`user_groups`）；每位使用者最多屬於一個群組，或未分組 |
| 群組配額 (Group quota) | 群組上的 `max_urls` / `max_api_tokens`。`NULL` 表示跟隨全站設定、`0` 表示無上限、正數則為上限 |
| 功能開關 (Feature toggle) | 群組上的 `allow_*` 布林值，決定某項連結功能可否使用；關閉開關不會刪除既有資料 |
| 預設群組 (Default group) | `is_default = 1` 的唯一群組；新註冊與未指定群組的邀請碼會進到這裡。標記可轉移，但該群組不可刪除 |
| 生效額度 (Effective limits) | 實際套用到使用者的配額與開關：群組有設定就用群組值，否則用全站預設 |
| 主題 (Theme) | 視覺識別。內建主題是 Askama 模板覆寫目錄加上一份內嵌 CSS；上傳主題是管理員提供的 CSS 檔，存放於持久化 volume，並以基底模板呈現 |
| 預設主題 (Default theme) | 訪客沒有 cookie，或 cookie 指向已停用/未知主題時所使用的唯一已啟用主題 |
| 主題目錄 (Theme catalogue) | 六個內建主題（Verdant、Aurora、Paper、Signal、Gilded、Swiss）加上管理員上傳（`custom-…`） |
| 中轉頁 (Splash page) | `redirect_mode` 為 `splash` 的連結所顯示的中間頁；呈現選填的宣傳文案、目的地主機與倒數，之後再前往目標 |
| 轉址方式 (Redirect mode) | 連結如何把訪客交給目標：`direct`（立刻回 30x）或 `splash`（先顯示中轉頁）。儲存在 `urls.redirect_mode` |
| 轉址方式政策 (Redirect-mode policy) | 群組上的 `redirect_mode_policy`（`direct`、`splash` 或 `choice`），限制**新**連結可用的模式；既有連結維持原模式 |
| 公開短網址工具 (Public shortener) | 導覽首頁在 `/` 提供的縮網址表單；表單以 `POST /` 送出，也可透過 `POST /api/v1/public/urls` 使用 |
| 存取層級 (Access level) | `public_create_access` 設定（`guest`、`user` 或 `admin`），決定誰可以建立連結。層級為累加關係：`user` 包含管理員，`guest` 則兩者皆包含 |
| 訪客帳號 (Guest account) | 唯一保留的系統帳號 `users` 資料列（`is_system = 1`、`guest@urlet.invalid`），擁有匿名訪客建立的每個連結。它不出現在列表與計數中，無法登入也無法編輯 |
| 公開欄位開關 (Public field switch) | `public_allow_*` 布林值，決定公開表單是否提供某個選填欄位。送出時若填了被停用的欄位，伺服器端一律拒絕 |

---

## 3. 系統架構

### 3.1 架構圖

```
                        ┌─────────────────────────────────────────────────┐
                        │ urlet（單一容器）                                │
Browser / curl          │                                                  │
───────────────────►    │ Axum Router                                      │
:8080                   │  ├─ /               導覽首頁 + 公開短網址        │
                        │  │                    （POST / 建立連結）        │
                        │  ├─ /{slug}           跳轉 / 密碼 / 中轉頁       │
                        │  ├─ /api/v1/**        JSON API                   │
                        │  │   └─ /public/urls  未認證建立                 │
                        │  ├─ /app/**           SSR 使用者與管理介面       │
                        │  ├─ /app/lang/{code}  語言切換（302）            │
                        │  ├─ /app/theme/{key}  主題切換（302）            │
                        │  ├─ /docs             內嵌 API 參考頁            │
                        │  ├─ /assets/**        內嵌 CSS/JS                │
                        │  │   ├─ /themes/{key}          內建 CSS          │
                        │  │   ├─ /custom-themes/{key}   上傳 CSS          │
                        │  │   └─ /link-previews/{key}   預覽圖片          │
                        │  ├─ /health           健康檢查                   │
                        │  └─ /healthz          健康檢查別名               │
                        │                                                  │
                        │ Middleware: 安全標頭 → 請求 ID/日誌              │
                        │             → 網域閘門 → 主題 → 語言             │
                        │             → Session/Token 認證 → CSRF          │
                        │             → 速率限制 → 驗證碼                  │
                        │                                                  │
                        │ Services: slug / url / click / ua / ratelimit /  │
                        │           audit / settings / i18n / captcha      │
                        │                                                  │
                        │ sqlx Pool (WAL)  ──►  /data/urlet.db             │
                        └─────────────────────────────────────────────────┘
                         named volume: urlet-data
```

### 3.2 技術選型與理由

| 層級 | 選擇 | 理由（相對於先前半成品） |
|------|------|--------------------------|
| HTTP | `axum` 0.8 + `tokio` | 生態成熟、型別安全、單一二進位 |
| DB | `sqlx` 0.8 + SQLite（`bundled`） | 免外部服務、單檔備份、WAL 併發讀取足夠自託管規模 |
| 遷移 | `include_str!` 內嵌 SQL + 自製 runner | 編譯期嵌入、啟動自動遷移、`schema_migrations` 記錄 checksum，無需 CLI |
| 模板 | `askama` 0.12 | 編譯期檢查的 SSR，無 Node/Vite 建置流程 |
| 靜態資產 | `include_str!` 內嵌 CSS/JS | 二進位自帶資源，映像檔不需檔案系統 |
| i18n | 以 `include_str!` 內嵌 JSON 語言目錄（`src/i18n/mod.rs` + `src/i18n/{en,zh-Hant,zh-Hans}.json`） | 與二進位一同部署、無外部語言檔；缺少 key 時回退英文 |
| 防機器人驗證 | 可選 Cloudflare Turnstile 或 Google reCAPTCHA v2（HTTPS 驗證），透過 `ureq` 2 + rustls + webpki-roots | 預設關閉、不綁定特定廠商、不需 OpenSSL、失敗一律拒絕 |
| 密碼 | `argon2` (Argon2id) | 業界標準，抗 GPU 暴力破解 |
| Session/Token | 自製 `sha2` 雜湊 + 隨機 `getrandom` | 不引入 JWT 相依；伺服器端可即時撤銷 |
| 日誌 | `tracing` + `tracing-subscriber` | 結構化日誌、`RUST_LOG` 控制 |
| 時間 | `time` 0.3 + `jiff` 0.2 | UTC 儲存與格式化，加上支援日光節約時間的伺服器自然週期配額 |

**刻意避免的相依**：Redis、MySQL client、jsonwebtoken、MaxMind/GeoIP、React/Node 工具鏈。
系統不會對外抓取目標網址（同時消除 SSRF 風險）；唯一的對外 HTTP 呼叫是可選的驗證碼
siteverify 請求（`src/services/captcha.rs`）：使用 `ureq` 2 搭配 rustls + webpki-roots
（純 Rust，因此 `rust:1-alpine` musl 建置不需要 OpenSSL），除非管理者設定供應商否則預設關閉，
**逾時 5 秒**，且任何失敗都一律拒絕（fail closed）。

### 3.3 二進位檔與映像檔尺寸目標

| 項目 | 目標 | v1.0 | v1.1 | v1.6 |
|------|------|------|------|------|
| Release 二進位（stripped, LTO, `opt-level=z`） | ≤ 15 MB | 4.2 MB（glibc）／4.5 MB（musl） | **5.7 MB**（glibc 本機）／**5.8 MB**（容器內 musl） | **15.7 MB**（容器內 musl） |
| 執行期映像（`alpine` + 二進位 + CA 憑證） | ≤ 40 MB | 13.2 MB | **14.8 MB** | **37.2 MB**（35.5 MiB） |
| 閒置／負載記憶體 | ≤ 30 MB RSS | 6.8 MB（500 次請求後） | **8.7 MB**（500 次請求後） | 未重新量測 |
| 冷啟動到可服務 | ≤ 1 秒 | 約 0.2 秒（含遷移） | **約 0.1 秒**（全新資料庫，含遷移） | 未重新量測 |

v1.1 欄已包含內嵌語言目錄與以 rustls 實作的驗證碼用戶端，v1.1 各項目標仍然達成。

v1.6 欄是在 `master` 加上內嵌 API 參考後量測。自 v1.1 以來的大部分成長來自其間新增的功能
——SNS 預覽圖片、公開短網址工具與主題系統——而非參考頁本身：同一棵樹移除參考頁後為
30.4 MB。參考頁增加 6.5 MiB，其中內嵌的 Scalar 前端佔 4.9 MB、兩份 OpenAPI 合約佔 0.4 MB。
二進位檔已超出 15 MB 目標 0.7 MB，而 37.2 MB 的映像仍在 40 MB 目標之內；若需要縮減映像，
槓桿在內嵌前端——壓縮後約 1.3 MB，改用 RapiDoc 之類較小的渲染器則為 0.5 MB，代價是失去
在地化的介面文字。

---

## 4. 功能需求

### 4.1 使用者與認證 (FR-AUTH)

| ID | 需求 | 說明 |
|----|------|------|
| FR-AUTH-1 | 註冊 | Email + 密碼。註冊模式由設定 `registration_mode` 控制：`open` / `invite` / `closed`。`GET /app/register?invite=CODE` 會預填邀請碼欄位，註冊連結可直接使用 |
| FR-AUTH-2 | 登入 | Email + 密碼，成功後建立 Session（HttpOnly Cookie，預設 TTL 7 天）。每次活動（最多每分鐘一次寫入）同時更新 `last_seen_at` 與 `expires_at`，並重新簽發 Cookie，因此持續使用不會被登出 |
| FR-AUTH-3 | 登出 | 撤銷當前 Session（`revoked_at`）並清除 Cookie |
| FR-AUTH-4 | 修改密碼 | 需驗證當前密碼；成功後撤銷該使用者其他所有 Session |
| FR-AUTH-5 | 個人資料 | 修改顯示名稱；檢視 Email、角色、建立時間 |
| FR-AUTH-6 | Session 管理 | 檢視自己的登入裝置清單，可撤銷任一 Session |
| FR-AUTH-7 | 首個使用者 | 若資料庫無任何使用者，第一位註冊者自動成為 `admin`；亦可用 `URLET_ADMIN_EMAIL/PASSWORD` 於啟動時建立 |
| FR-AUTH-8 | 停權 | `status = suspended` 的使用者無法登入，既有 Session 立即失效 |
| FR-AUTH-9 | 速率限制 | 登入/註冊/解鎖具記憶體令牌桶限制，並記錄 `login_attempts` 供稽核 |

**密碼政策**：長度 8–128 字元；Argon2id 參數 `m=19456 KiB, t=2, p=1`（OWASP 建議值）。

### 4.2 短網址管理 (FR-URL)

| ID | 需求 | 說明 |
|----|------|------|
| FR-URL-1 | 建立 | 必填 `target_url`；可選 `slug`、標題、備註、標籤、密碼、過期時間、點擊上限、轉址類型、轉址方式與中轉頁文案 |
| FR-URL-2 | 自動 Slug | 未指定時產生 base62 隨機 Slug，長度為 `default_slug_length` 設定（4–16，預設 7），碰撞時重試（最多 5 次） |
| FR-URL-3 | 自訂 Slug | 長度 1–64，僅 `[A-Za-z0-9_-]`，不得為保留字，全域唯一 |
| FR-URL-4 | 編輯 | 可改 target、標題、備註、標籤、密碼（設定/清除）、過期、點擊上限、轉址類型、轉址方式、中轉頁文案與狀態 |
| FR-URL-5 | 刪除 | 軟刪除（`deleted_at`），Slug 保留不釋出（避免連結被劫持），點擊紀錄保留 |
| FR-URL-6 | 列表 | 分頁、關鍵字搜尋（slug/target/標題）、標籤篩選、狀態篩選、排序（建立時間/點擊數） |
| FR-URL-7 | 狀態切換 | `active` / `disabled`；`disabled` 不轉址（回 404 頁面） |
| FR-URL-8 | 標籤 | 使用者層級標籤，名稱唯一（同使用者內），建立網址時自動建立缺少的標籤 |
| FR-URL-9 | 配額 | 每位使用者網址數量上限為 `max_urls_per_user` 的生效值：使用者所屬群組的 `user_groups.max_urls` 有設定時優先採用，否則使用全站設定（0 = 無限制；FR-GROUP-2） |
| FR-URL-10 | 所有權隔離 | 所有 `/api/v1/urls*` 查詢強制 `user_id = current_user`；他人資源一律 404 |

### 4.3 密碼保護 (FR-PWD)

| ID | 需求 | 說明 |
|----|------|------|
| FR-PWD-1 | 設定 | 建立/編輯時提供明文密碼，系統以 Argon2id 雜湊存入 `urls.password_hash` |
| FR-PWD-2 | 存取 | 受保護的短網址直接訪問時回傳 200 + 密碼輸入頁（**不洩漏目標網址**） |
| FR-PWD-3 | 解鎖 | `POST /{slug}/unlock` 驗證密碼；成功後設定簽章 Cookie `urlet_unlock_<slug>`（HMAC-SHA256，TTL 30 分鐘） |
| FR-PWD-4 | 已解鎖 | 持有效解鎖 Cookie 時直接轉址，並記錄 `password_protected = 1` |
| FR-PWD-5 | 速率限制 | 每個 (slug, IP) 5 分鐘內最多 10 次嘗試，超過回 429 |
| FR-PWD-6 | 錯誤處理 | 密碼錯誤回 401 並在頁面上顯示錯誤，不揭露目標網址 |

### 4.4 過期機制 (FR-EXP)

| ID | 需求 | 說明 |
|----|------|------|
| FR-EXP-1 | 時間過期 | `expires_at`（ISO-8601 UTC）。到達後短網址失效，回 **410 Gone** 頁面 |
| FR-EXP-2 | 點擊上限 | `max_clicks` 非空且 `click_count >= max_clicks` 時失效，回 410。計數器由背景 writer 非同步累加，因此並發請求可能略微超出上限（已知取捨：換取轉址路徑零寫入阻塞） |
| FR-EXP-3 | 統計可見性 | 使用者介面以「已過期 / 已達上限」徽章標示，並可一鍵延長或提高上限 |
| FR-EXP-4 | 過期不刪除 | 過期僅影響轉址，資料與統計保留 |
| FR-EXP-5 | 已刪除 | `deleted_at` 非空 → 404（與不存在無法區分） |
| FR-EXP-6 | 停用 | `status = disabled` → 404 |

### 4.5 轉址與點擊記錄 (FR-REDIR)

| ID | 需求 | 說明 |
|----|------|------|
| FR-REDIR-1 | 路由 | `GET /{slug}`；`/api/**`、`/app/**`、`/assets/**`、`/health` 為保留前綴 |
| FR-REDIR-2 | 轉址類型 | 支援 301 / 302（預設）/ 307 / 308 |
| FR-REDIR-3 | 點擊記錄 | 非同步（`tokio::spawn`）寫入 `clicks` 與 `url_daily_stats`，不阻塞轉址 |
| FR-REDIR-4 | 使用者代理分析 | 解析 device_type / browser / os；機器人（bot UA）標記 `is_bot = 1` |
| FR-REDIR-5 | 隱私 | 僅存 `ip_hash = HMAC(secret, ip)` 與 `visitor_hash = HMAC(secret, "{ip_hash}\|{user_agent}\|{day}")`（HMAC 的材料是已雜湊的 IP，不是原始位址）；**不存原始 IP** |
| FR-REDIR-6 | 唯一訪客 | 同一 `visitor_hash` 於同一 URL 首次點擊記 `is_unique = 1` |
| FR-REDIR-7 | 快取控制 | 密碼頁／中轉頁／410／404 為 `no-store`；302/307 轉址為 `private, no-store`；301/308 轉址為 `private, max-age=300`（永久轉址可短期快取，其餘一律不快取以維持點擊準確） |
| FR-REDIR-8 | 統計開關 | 設定 `analytics_enabled = false` 時不寫入點擊 |
| FR-REDIR-9 | 轉址方式 | `urls.redirect_mode` 為 `direct` 或 `splash`。中轉頁連結改回 200 中間頁而不是 30x；點擊在中間頁送出時記錄。受密碼保護的連結不會顯示中轉頁（FR-SPLASH-4） |

### 4.6 統計與分析 (FR-STAT)

| ID | 需求 | 說明 |
|----|------|------|
| FR-STAT-1 | 總覽 | 我的網址數、總點擊、近 7/30 日點擊、唯一訪客、機器人佔比 |
| FR-STAT-2 | 單一網址 | 每日點擊趨勢（近 30 日）、Top 來源網域、裝置/瀏覽器/作業系統分佈、最近 50 筆點擊 |
| FR-STAT-3 | 圖表 | 伺服器端渲染 SVG（無前端圖表函式庫） |
| FR-STAT-4 | 資料保留 | `click_retention_days`（預設 0 = 永久）；> 0 時由每日維護任務清理 |

### 4.7 API Token (FR-TOK)

| ID | 需求 | 說明 |
|----|------|------|
| FR-TOK-1 | 建立 | 產生 `ur_` + 32 bytes CSPRNG 熵（base64url 編碼，43 字元）；**僅在建立時顯示一次**明文 |
| FR-TOK-2 | 儲存 | 僅存 SHA-256 雜湊 + 前 8 字元前綴（供辨識） |
| FR-TOK-3 | 使用 | `Authorization: Bearer <token>`；與 Session 認證並存 |
| FR-TOK-4 | 生命週期 | 可設 `expires_at`、可撤銷（`revoked_at`）、記錄 `last_used_at`；使用者改密碼、管理員重設密碼或刪除帳號時，該帳號所有 Token 一併撤銷 |
| FR-TOK-5 | 範圍 | `scopes` 欄位保留（v1 提供 `urls:read`、`urls:write`、`admin`），v1 僅檢查是否為 admin |
| FR-TOK-6 | 配額 | 每位使用者可用 Token 數（未撤銷且未過期）受生效的 `max_api_tokens` 限制；全站預設為 `max_api_tokens_per_user=20`（0 = 無限制），群組可覆蓋或跟隨（FR-GROUP-2） |
| FR-TOK-7 | 功能開關 | 群組 `allow_api_tokens = 0` 時，帳號頁與 API 建立端點一律拒絕發行 Token（403）；既有 Token 會繼續運作，直到管理員或使用者手動撤銷 |

### 4.8 管理後台 (FR-ADMIN)

| ID | 需求 | 說明 |
|----|------|------|
| FR-ADMIN-1 | 使用者管理 | 列表、搜尋（含依群組）、改角色、指派群組、停權/復權、重設密碼、軟刪除 |
| FR-ADMIN-2 | 全站網址 | 列表、搜尋、強制停用/刪除 |
| FR-ADMIN-3 | 邀請碼 | 建立（次數/到期/備註/可選目標群組）、列表、撤銷；`invite` 模式下註冊需邀請碼。指定群組的邀請碼會在兌換時將使用者加入該群組（FR-GROUP-4）。邀請碼與註冊連結（`/app/register?invite=CODE`）在建立後仍可查看；v1.3 之前建立或密鑰更換後的加密封存會回退為前綴。撤銷從未被兌換的邀請碼會退回配額，已被兌換的邀請碼則維持計入 |
| FR-ADMIN-4 | 全站設定 | 編輯全站設定。可編輯鍵的權威清單位於 [`docs/API.md`](API.md) |
| FR-ADMIN-5 | 稽核日誌 | 記錄登入、註冊、網址異動、群組異動、管理操作、設定變更；可依動作/對象篩選 |
| FR-ADMIN-6 | 系統資訊 | 版本、資料庫大小、使用者/網址/點擊總數、運行時間 |
| FR-ADMIN-7 | 授權 | 所有 `/api/v1/admin/**` 與 `/app/admin/**` 需 `role = admin`，否則 403 |
| FR-ADMIN-8 | 群組管理 | 於 `/app/admin/groups` 與 `/api/v1/admin/groups` 建立、編輯、設為預設、設定網域策略與刪除使用者群組（FR-GROUP-1、FR-GROUP-9） |
| FR-ADMIN-9 | 主題管理 | 於 `/app/admin/themes` 與 `/api/v1/admin/themes` 啟用/停用內建與上傳主題、上傳/編輯/刪除自訂樣式表、選擇唯一預設並設定誰能切換（FR-THEME-2、FR-THEME-10） |

### 4.9 JSON API (FR-API)

- 前綴 `/api/v1`，回應 `application/json`。
- 統一錯誤格式：
  ```json
  { "error": { "code": "validation_error", "message": "slug is invalid", "details": { "field": "slug" } } }
  ```
- 錯誤代碼：`bad_request` `unauthorized` `forbidden` `not_found` `conflict` `gone` `validation_error` `captcha_failed` `rate_limited` `internal_error`
- 分頁格式：`{ "items": [...], "page": 1, "per_page": 20, "total": 42, "total_pages": 3 }`
- 認證：Session Cookie **或** `Authorization: Bearer <api_token>`。
- **JSON API 回應一律為英文**，不受請求語言或 `urlet_lang` cookie 影響。
- 執行個體自己提供可瀏覽的參考頁，內嵌於執行檔中，因此沒有對外網路的執行個體也能使用：
  - `GET /docs` 以訪客的介面語言呈現參考頁。
  - `GET /docs/openapi.json` 與 `GET /docs/openapi.zh-TW.json` 提供 OpenAPI 3.1 合約。
    兩者都由 `scripts/build-openapi.py` 從 `openapi/` 產生後提交，因為執行檔以
    `include_str!` 內嵌它們；CI 會重新產生並在有任何差異時失敗。
  - 兩份合約必須描述同一組操作，且每個操作都必須參照文件宣告的分組。分組名稱與其餘敘述
    一併翻譯，因此參考頁的側邊欄會以訪客的語言呈現。簡體中文訪客取得英文合約，因為
    敘述性文件只有雙語。
  - Scalar 內嵌於 `src/assets/scalar.js`，由 `scripts/update-scalar.sh` 釘住版本與雜湊。
    該頁面不會發出任何第三方請求，也不需要 CSP 例外：掛載元素避開了 Scalar 會自動偵測的
    id，因此傳給它的選項得以生效。
  - 合約以設定的 base URL 作為 server，參考頁也只存在於該主機。頁尾會在每個設計（含額外
    網域的簡短首頁）連結它，但在額外網域上 `/docs` 與 `/docs/*` 會回 `308` 轉往 canonical
    主機，而不是提供該頁面：參考頁的 Test Request 會呼叫 canonical base URL，而 CSP 只允許
    `connect-src 'self'` 且沒有 CORS 層，因此在額外網域提供的互動式參考頁，每個請求都會被
    瀏覽器擋下。base URL 屬於部署設定，因此產生器在提交的合約中留下佔位符，由回應在提供時
    替換；該值不隨請求變動，因此每個主機取到的快取內容相同。
  - `POST /{slug}/unlock` 是例外：每個已啟用的網域都提供它，且 slug 是在請求抵達的網域上
    解析，因此這個操作以連結自己的來源作為 server 變數，而非繼承 canonical API server。
  - `/docs` 是保留 slug：短網址不得佔用。已存在且持有該 slug 的連結會在啟動時回報，
    並保留給擁有者自行重新命名。
- 合約與 router 由測試綁在一起，因為型別系統沒有任何東西連接它們。axum 不提供讀回已建
  router 的方式，因此 `tests/contract.rs` 直接從 router 的原始碼讀取表面——`.route(...)`
  呼叫就是註冊本身——並與兩份合約的 paths 與 methods 做集合相等比對。新增 handler 卻忘了
  加合約條目會失敗，合約條目對應的 handler 消失也會失敗。解析器遇到不理解的形狀會直接
  失敗，並有大小守門，讓「什麼都沒匹配到」的解析器失敗而不是僥倖通過。CI 另外會 lint
  兩份產生的文件——這正是抓出四個「描述的是儀表板 payload 而非 API 回應」的元件的原因。
- 同一批文件也以靜態、可搜尋的網站形式發布：原始碼位於 `docs-site/`，由一個以路徑過濾的
  workflow 部署到 `gh-pages` 分支。repo 的 `docs/` 樹仍是唯一來源：prebuild 步驟將它複製
  進網站的內容樹、從每個檔案自己的第一個標題推導頁面標題，並改寫文件之間的連結。產生的
  副本已被 gitignore，因此 repo 不會帶著第二份可能漂移的副本。網站是獨立產物，不會進入
  Docker context 或執行檔。
- 端點清單見 §7。

### 4.10 網頁介面 (FR-UI)

| 路徑 | 頁面 | 說明 |
|------|------|------|
| `/` | 導覽首頁 + 公開短網址工具 | hero、功能亮點、登入與註冊入口、語言切換、帶上游 repo 連結的頁尾（FR-THEME-11）；已登入者另顯示進入儀表板的入口。當存取層級允許訪客建立連結時，頁面也會帶上公開短網址工具表單與上次送出的結果。不再是 302 轉到 `/app` |
| `/app/login` | 登入 | Email + 密碼 |
| `/app/register` | 註冊 | 依註冊模式顯示邀請碼欄位或關閉提示；`?invite=CODE` 會預填欄位 |
| `/app` | 儀表板 | 統計卡片、每日趨勢圖、網址列表（搜尋/篩選/分頁）、建立表單（當存取層級把建立權保留給管理員時，改以提示取代） |
| `/app/urls/{id}` | 網址統計 | 連結設定摘要、統計圖表、最近點擊與快速延長操作；「編輯設定」按鈕以懸浮窗開啟編輯器（QR 免，Non-Goal） |
| `/app/urls/{id}/edit` | 網址編輯器 | 共用的連結表單與刪除。儀表板與統計頁將它載入懸浮窗；無 JavaScript 時這一頁就是完整的編輯器。儲存被拒時以送出的值重新渲染（`422`） |
| `/app/account` | 帳號 | 個人資料、修改密碼、Session 清單、API Token 管理 |
| `/app/admin` | 管理後台 | 使用者、使用者群組、全站網址、網域、邀請碼、設定、稽核日誌 |
| `/app/admin/domains` | 網域管理 | 新增、啟用、停用、設為預設、刪除網域 |
| `/app/admin/groups` | 使用者群組 | 列出群組及其配額、邀請碼用量、政策與成員數；連往新增與編輯表單 |
| `/app/admin/groups/new` | 新增群組 | 建立表單：配額、功能開關、網域政策、轉址方式政策與預設網域；群組建立後才能轉移預設標記 |
| `/app/admin/groups/{id}/edit` | 編輯群組 | 與 `/app/admin/groups/new` 相同的表單，預填目前設定；刪除與成員轉移在此頁 |
| `/app/lang/{code}` | 語言切換 | 設定 `urlet_lang` cookie 並 302 至 `next`（未帶時轉往 `/app`） |
| `/app/theme/{key}` | 主題切換 | 需要切換權限：已登入帳號，或 `all` 政策下的未登入訪客；設定 `urlet_theme` cookie 並轉往同站 `next`（或 `Referer`，已登入帳號為 `/app`、訪客為 `/`）。`admin` 政策下訪客改為導向登入頁。`?preview=1` 設定管理員專用預覽 cookie，可包含已停用主題 |
| `/app/theme/default` | 網站預設 | 清除明確選擇與預覽 cookie，重新跟隨管理員預設；權限與切換相同 |
| `/app/theme/preview/stop` | 結束預覽 | 清除管理員預覽 cookie |
| `/app/admin/themes` | 主題管理 | 啟用/停用內建與上傳主題、選擇預設與設定 `theme_switching` 政策；連往新增與編輯表單 |
| `/app/admin/themes/new` | 新增自訂主題 | 上傳表單：名稱、說明、外觀與樣式表；與編輯頁共用同一個模板 |
| `/app/admin/themes/custom` | 自訂主題上傳 | `POST` multipart 上傳（名稱、說明、外觀、CSS）；僅限管理員 |
| `/app/admin/themes/custom/{id}` | 自訂主題編輯 | `GET` 共用表單並預填目前設定、`POST` 更新中介資料與選填的樣式表替換 |
| `/app/admin/themes/custom/{id}/delete` | 自訂主題刪除 | `POST` 移除目錄資料列與 CSS 檔；預設主題不可刪除 |
| `/{slug}` | 轉址 / 密碼頁 / 中轉頁 / 410 / 404 | 公開 |
| 額外網域 | 僅轉址 | `/` 顯示簡單首頁、`/{slug}` 轉址、`/app` 308 回 canonical、`/api` 回 404 |

**UI 技術**：Askama 模板 + 手寫 CSS（含 `prefers-color-scheme` 深色模式）+ 約 500 行原生 JS
（複製到剪貼簿、確認對話框、表單增強、群組與主題表單共用的新增/編輯懸浮框）。
**不使用**任何前端框架或 CDN 資源（CSP 可嚴格限制為 `'self'`）。

### 4.11 多語系介面 (FR-I18N)

| ID | 需求 | 說明 |
|----|------|------|
| FR-I18N-1 | 支援語言 | 介面提供三種語言：英文（`en`，預設）、繁體中文（`zh-Hant`）、簡體中文（`zh-Hans`）。語言目錄為內嵌於二進位檔的 JSON 檔 `src/i18n/{en,zh-Hant,zh-Hans}.json`，由 `src/i18n/mod.rs` 載入 |
| FR-I18N-2 | 語言選擇 | 初次訪問（無 cookie）時依 `Accept-Language` 協商，正確處理 `q=` 權重並由高至低排序；`zh-TW`、`zh-HK`、`zh-MO` 對應 `zh-Hant`，其餘中文標籤（含單獨的 `zh`）對應 `zh-Hans`，無法辨識或未帶標頭時預設 **英文**。`urlet_lang` cookie 一律優先於標頭 |
| FR-I18N-3 | 保存 | 選擇的語言存於 cookie `urlet_lang`（`Path=/`、`SameSite=Lax`、`Max-Age` 一年、**非** HttpOnly；`URLET_SECURE_COOKIES=true` 時加上 `Secure`） |
| FR-I18N-4 | 切換 | `GET /app/lang/{code}?next=<path>` 設定 cookie 後回 302；`next` 只接受同站相對路徑，其餘（含未帶 `next`）一律回退 `/app`。無法辨識的 `code` 一律視為 `en`，不會回錯誤 |
| FR-I18N-5 | 標記與標頭 | `<html lang>` 會反映當前語言；公開頁面與應用外框皆提供語言切換入口；HTML 回應會帶 `Content-Language` 與 `Vary: Accept-Language, Cookie`（靜態資源不設 `Vary`） |
| FR-I18N-6 | 回退 | 語言目錄缺少的 key 回退使用英文字串 |
| FR-I18N-7 | API 回應 | 所有 JSON API 回應一律英文，不受請求語言或 cookie 影響 |
| FR-I18N-8 | 錯誤頁面 | 錯誤頁面（404/410/500 等）透過 task-local 語言 context 依當前語言呈現 |

### 4.12 防機器人驗證 (FR-BOT)

| ID | 需求 | 說明 |
|----|------|------|
| FR-BOT-1 | 供應商 | 可選 Cloudflare Turnstile（元件回應欄位 `cf-turnstile-response`）或 Google reCAPTCHA v2 checkbox（`g-recaptcha-response`），預設 `none`（關閉） |
| FR-BOT-2 | 設定來源 | 同時支援環境變數與管理後台設定（settings 表新增 key：`captcha_provider`、`captcha_site_key`、`captcha_secret_key`、`captcha_on_register`、`captcha_on_login`、`captcha_on_create`、`captcha_allowed_hostnames`）。非空環境變數覆蓋後台值，與 `registration_mode` 的覆蓋規則一致 |
| FR-BOT-3 | 啟用條件 | 四個條件同時成立才啟用：provider ≠ `none`、兩把 key 皆非空、對應表單的開關（`on_register`、`on_login` 或 `on_create`）為 `true` |
| FR-BOT-4 | 驗證方式 | 伺服器端以表單編碼（form-encoded）HTTPS POST 呼叫供應商的 siteverify 端點（帶 `secret`、`response`，以及可判定時的 `remoteip`）：Cloudflare `https://challenges.cloudflare.com/turnstile/v0/siteverify`、Google `https://www.google.com/recaptcha/api/siteverify`，逾時 5 秒，並透過 `ureq` 2 + rustls + webpki-roots 發出 |
| FR-BOT-5 | 失敗關閉 | 任何失敗（逾時、非 2xx、回應無法解析）一律拒絕（fail closed） |
| FR-BOT-6 | 套用範圍 | 網頁註冊表單、網頁登入表單、網頁建立短網址表單，以及 `POST /api/v1/auth/register` 與 `POST /api/v1/auth/login`（JSON 欄位 `captcha_token`，失敗回 400 `captcha_failed`）。`POST /api/v1/urls` 的 Session Cookie 呼叫見 FR-BOT-7 |
| FR-BOT-7 | API Token 路徑 | `POST /api/v1/urls` **僅在**帶 API Token（`Authorization: Bearer ur_...`）時豁免驗證碼。使用 Session Cookie 者屬瀏覽器工作階段，受 `on_create` 約束，可帶 optional `captcha_token` |
| FR-BOT-8 | 後台介面 | secret key 欄位為唯寫：不回顯已存值，留白送出表示維持原值 |
| FR-BOT-9 | Token 主機名稱 | 供應商 siteverify 回應中的 `hostname` 必須等於 `URLET_BASE_URL` 的主機名稱，或 `captcha_allowed_hostnames` 中的任一個（逗號分隔的裸主機名稱，逐項 trim、轉小寫、丟棄空項，最多 10 項、每項 1–253 字元且不得含空白、`/`、`:`、`@`）。回應未帶 `hostname` 或該欄位為空字串時予以接受；不符時拒絕並記一筆 `WARN`，內容含回報的 hostname 與目前接受的候選清單 |

### 4.13 使用者群組 (FR-GROUP)

| ID | 需求 | 說明 |
|----|------|------|
| FR-GROUP-1 | 群組 CRUD | 管理員可建立、改名、編輯與刪除群組。名稱 trim 後長度 1–40 字元、以 `COLLATE NOCASE` 不分大小寫唯一、不得含控制字元；說明為選填且最多 200 字元。配額可為 `NULL`（跟隨全站）、`0`（無上限）或 1–1,000,000 的正整數 |
| FR-GROUP-2 | 配額繼承 | 生效的 `max_urls` / `max_api_tokens` = 群組值非 `NULL` 時採群組值，否則採 `settings.max_urls_per_user` / `settings.max_api_tokens_per_user`；未分組使用者一律使用全站值 |
| FR-GROUP-3 | 功能開關 | 七個布林值，預設全為 `true`：`allow_custom_slug`、`allow_password`、`allow_expiry`、`allow_max_clicks`、`allow_tags`、`allow_api_tokens`、`allow_domain_selection`。它們在建立時把關；更新時拒絕「新開啟」被封鎖的功能，但允許清除或省略欄位以維持原值。生效的群組預設網域一律允許；`allow_domain_selection = false` 會拒絕建立或移動連結到其他網域（FR-GROUP-9）。非空的連結密碼一律視為新值，在 `allow_password` 關閉時會被拒絕。既有連結與 Token 不會因關閉開關而被修改或刪除 |
| FR-GROUP-4 | 歸屬 | 每位使用者最多屬於一個群組（`users.group_id`，可為空）。註冊時指派預設群組；邀請碼可指定群組，兌換時優先於預設群組。在 `open` 模式下提供的邀請碼同樣會驗證，因此其群組仍會生效。管理員可改派任何使用者；因為每次請求都重新載入使用者，改派立即生效 |
| FR-GROUP-5 | 預設群組 | 恰有一筆 `is_default = 1`，由部分唯一索引保證。可於群組編輯表單轉移標記（同一交易內取消舊預設）；目前為預設的群組不可刪除。遷移時建立名為 `Standard`、兩個配額皆為 `NULL` 的群組，並把所有既有使用者指派進去，因此升級後行為不變 |
| FR-GROUP-6 | 刪除 | 刪除群組必須指定改派目標（API 用 `reassign_to`，網頁用下拉選單）。同一交易內把所有成員搬到目標群組；指向該群組的邀請碼透過 `ON DELETE SET NULL` 回退到預設群組。改派到即將刪除的群組本身、或刪除預設群組，一律回 409 |
| FR-GROUP-7 | 強制範圍 | 配額與開關對管理員一體適用。調降配額或關閉開關不會刪除或停用既有連結與 Token，只會擋下新的建立（以及更新時新開啟被封鎖的功能） |
| FR-GROUP-8 | 可見性 | 帳號頁顯示群組名稱、連結用量 `已用 / 配額` 與 Token 用量 `已用 / 配額`；`GET /api/v1/auth/me` 回傳 `group` 與 `limits`（生效配額、開關、轉址方式政策、預設網域與允許網域），讓 API 客戶端能預先判斷請求是否會被拒絕 |
| FR-GROUP-9 | 網域策略 | `user_groups.default_domain_id` 是群組預設網域（`NULL` = 全站預設）；未指定網域的新連結會使用它，若該網域不存在、停用或刪除則回退全站預設。`restrict_domains = 1` 時只允許 `group_domains` 內的網域與生效預設；開啟限制但沒有資料列時只能使用預設。`allow_domain_selection = false` 時也只能使用群組預設。既有連結維持目前網域一律允許；未分組使用者使用全站預設與所有 active 網域。 |
| FR-GROUP-10 | 轉址方式政策 | `user_groups.redirect_mode_policy` 為 `direct`、`splash` 或 `choice`，預設 `choice`，因此升級後行為不變。建立連結時使用被封鎖的模式、或把既有連結切換到該模式，回 `403`；維持目前模式與清除中轉頁文案則可接受。政策只影響新連結與模式切換，永不改寫既有行為（FR-GROUP-7） |

### 4.14 可切換主題 (FR-THEME)

| ID | 需求 | 說明 |
|----|------|------|
| FR-THEME-1 | 主題目錄 | binary 內建六個主題：`verdant`（v1.2 預設）、`aurora`、`paper`、`signal`、`gilded`、`swiss`。每個主題擁有 `templates/themes/<key>/` 模板目錄與內嵌 CSS。管理員另可註冊 `custom-…` 主題：它們是 CSS 檔、沿用 Verdant 模板，永不新增或修改模板 |
| FR-THEME-2 | 管理 | `/app/admin/themes` 與 `GET/PUT /api/v1/admin/themes` 列出所有主題（內建與上傳）、啟用/停用、選擇唯一預設與設定 `theme_switching`。上傳主題可於管理頁建立、編輯與刪除。至少要有一個啟用主題，且預設必須啟用；違反時回 `422 validation_error`。預設標記由 partial unique index 保護 |
| FR-THEME-3 | 選擇 | `GET /app/theme/{key}` 需要切換權限，僅接受已啟用的 key，設定 `urlet_theme` cookie（`Path=/`、`SameSite=Lax`、`HttpOnly`、一年，設定時加 `Secure`），並轉往同站 `next`、否則 `Referer` 路徑、否則已登入帳號為 `/app`、訪客為 `/`。未知或已停用的 key 回 404；無權限的訪客帶著 `next` 導向登入頁。`?preview=1` 設定獨立的管理員預覽 cookie，可指向已停用主題；`/app/theme/default` 清除兩種 cookie，`/app/theme/preview/stop` 結束預覽 |
| FR-THEME-4 | 權限 | `theme_switching` 為 `all`（預設）或 `admin`。`all` 就是字面上的每個人：已登入成員與未登入訪客，在具備站台外框的頁面（首頁、登入與註冊、錯誤頁）都看得到切換器並可切換。政策在頁面渲染時強制執行：設為 `admin` 時，非管理員帳號與未登入訪客一律顯示預設、忽略既有 cookie、不提供切換器，非管理員的直接切換回 403，訪客則導向登入頁。管理員永遠看得到切換器，並可透過獨立的預覽 cookie 檢視任何已編譯主題（包含已停用者）。轉址介面（短連結、密碼與 splash 頁）永遠不提供切換器，只跟隨 cookie。停用主題不會刪除資料 |
| FR-THEME-5 | 預設與回退 | 恰有一個已啟用主題標記為預設。沒有 cookie、未知 key 或 key 已被停用時一律使用預設；切換器依 migration 的順序列出已啟用主題，並提供會清除 cookie 的**網站預設**項目；管理員可在不改變預設的情況下預覽已停用主題 |
| FR-THEME-6 | 模板 | `templates/` 下的基底模板即 Verdant 主題與回退。主題目錄可覆寫任一頁面；未覆寫的頁面與共享的 v1.2 頁面內容仍可使用。主題模板由 `themed_page!` 在編譯期選擇，絕不使用請求提供的路徑 |
| FR-THEME-7 | 資產 | `/assets/themes/{key}` 只提供固定的內建 key；`/assets/custom-themes/{key}` 依資料庫資料列提供上傳樣式表（前面接上 Verdant 基礎 CSS，讓上傳能與預設設計疊加），URL 帶有 `/assets/custom-themes/{key}?v=<內容雜湊>`；`/assets/theme-ui.css` 提供切換器與管理目錄的共用樣式。不引入外部字型、圖片、腳本或 CDN，維持 `default-src 'self'` CSP 與零建置部署 |
| FR-THEME-8 | 外觀 | 除了 Aurora，每個內建設計都提供兩種色系與持久化的明暗控制：訪客選定模式之前跟隨 `prefers-color-scheme`，選定後 `data-theme` 兩個方向都生效，因此儲存的淺色偏好也能勝過深色的作業系統。Aurora 僅有深色，在 `<html>` 標記 `data-theme="dark"`／`data-theme-lock="dark"` 並隱藏控制項，而不是放一個無效的按鈕。目錄的 `appearance` 欄位與「僅淺色／僅深色」徽章都來自同一份 `ThemeKey` 政策，不寫死旗標。上傳主題有管理員選擇的外觀：`auto` 保留明暗控制，`light`／`dark` 透過 `data-theme`／`data-theme-lock` 強制調色盤、隱藏控制，且瀏覽器儲存的偏好無法覆寫 |
| FR-THEME-9 | 持久化 | 選擇存在 cookie，而非每位使用者的資料庫欄位。已停用的主題在下一個請求即被拒絕，管理員可隨時為沒有偏好的訪客更改預設。上傳主題的中介資料在 `custom_themes`、樣式表在持久化 volume 的 `URLET_THEMES_DIR`、目錄狀態在 `themes`，重啟後仍可選用。主題切換不觸碰任何連結、使用者或點擊資料 |
| FR-THEME-10 | 上傳 | 上傳、編輯與刪除僅限管理員。上傳內容為單一 UTF-8 `.css` 檔、最大 256 KiB；名稱 1–60 字元、說明最多 200 字元、外觀為 `auto`／`light`／`dark`；每個執行個體最多 32 個上傳。CSS 以 `text/css` 與 `nosniff` 提供，接在 Verdant 基礎樣式表之後，且不可使用外部 `url(...)`、`@import`、`expression(...)` 或舊式 `behavior`。替換樣式表會改變內容雜湊 URL；刪除會移除檔案；預設主題不可刪除 |
| FR-THEME-11 | 頁尾署名 | 每個設計的頁尾都連向上游 repo，讓自架執行個體能標明自己是從哪個專案建置的。URL 來自 `Cargo.toml` 的 `repository` 欄位，以 `urlet::REPOSITORY` 提供，並由 `templates/partials/repo_link.html` 渲染；每個 base 模板都納入它，包含 Gilded 獨立的首頁頁尾與額外網域的簡短首頁。標籤使用品牌名 `GitHub`，在每份 catalogue 都保持原文；圖示是 inline SVG，因為 CSP 不允許遠端圖片，且專案沒有圖示字型。連結以新分頁開啟並帶 `rel="noopener"`；共用的 `.footer-link` 規則放在 `/assets/theme-ui.css`（`.footer-repo` 保留為別名，讓已上傳且針對它撰寫的樣式表繼續運作），各主題的擺放位置與字級則緊鄰該設計自己的頁尾規則 |
| FR-THEME-12 | 參考頁入口 | 每個設計的頁尾也連向內嵌的 API 參考頁 `/docs`，由 `templates/partials/api_docs_link.html` 渲染，位置緊接在 repo 連結之前，讓署名維持在最後。可見標籤在每份 catalogue 都是 `API`，無障礙名稱則翻譯（`footer.api_label`），因此無障礙名稱仍包含可見標籤。連結不帶圖示：專案沒有圖示字型，而第二個 inline 標記會是多一個要同步的 owner。它使用共用的 `.footer-link` 規則，因此各主題的頁尾字級會原樣套用 |

### 4.15 中轉頁 (FR-SPLASH)

| ID | 需求 | 說明 |
|----|------|------|
| FR-SPLASH-1 | 模式 | 每個連結都有 `redirect_mode`：`direct`（預設，也就是既有行為）或 `splash`。建立與編輯表單及 `POST/PATCH /api/v1/urls` 皆接受此欄位；資料庫中的未知值在轉址路徑回退為 `direct` |
| FR-SPLASH-2 | 渲染 | 有效且未受密碼保護的中轉頁連結回 `200` HTML，並帶 `Cache-Control: no-store`。頁面顯示自訂（或在地化預設）標題與說明、目的地主機、倒數與「立即前往」按鈕，並以 `<meta http-equiv="refresh">` 自動轉往目標。除了共用同源網站資源外，不額外載入腳本或外部資源，因此維持嚴格 CSP。中轉頁對所有 User-Agent 一視同仁，因此連結預覽（unfurl）看到的是中轉頁而非目的地；這是刻意行為，且頁面帶 `noindex` |
| FR-SPLASH-3 | 點擊記錄 | 中轉頁曝光即為一次點擊，於頁面送出時排入佇列。meta refresh 與「立即前往」都直接連到目標，因此不會重複計算一次目標造訪 |
| FR-SPLASH-4 | 密碼優先 | 有密碼的連結一律顯示密碼頁，絕不顯示中轉頁。解鎖成功後直接前往目標；密碼頁是唯一的中間頁 |
| FR-SPLASH-5 | 文案 | `splash_title` 最多 120 字元且不得含控制字元。`splash_message` 最多 500 字元，允許換行、拒絕其他控制字元。兩者皆為選填；空白時使用在地化預設文案。標題與說明都會進行 HTML 跳脫 |
| FR-SPLASH-6 | 倒數 | `splash_delay_seconds` 為 `1..=60` 的整數，預設 `5`。超出範圍時表單與 API 皆拒絕；資料庫中的超範圍值在渲染時會被夾回範圍，避免手動改庫弄壞頁面 |
| FR-SPLASH-7 | 群組政策 | `user_groups.redirect_mode_policy` 為 `direct`（新連結必須直接跳轉）、`splash`（新連結必須使用中轉頁）或 `choice`（預設，兩者皆可）。它在建立與更新切換模式時把關，永不改寫既有連結；網頁表單會隱藏被封鎖的選項，被拒絕的請求回 `403`。政策透過 `GET /api/v1/auth/me` 的 `limits.redirect_mode_policy` 曝光 |

### 4.16 公開短網址工具 (FR-PUB)

| ID | 需求 | 說明 |
|----|------|------|
| FR-PUB-1 | 表單 | 只要訪客被允許建立連結，canonical 導覽首頁就會帶上縮網址表單；額外（僅轉址）網域永遠不會，且該處的 `POST /` 回 `404`。表單是一般的 HTML `POST /`，套用 CSRF 同源規則；送出成功後轉往 `/?shortened=<slug>&domain=<hostname>`。第一眼維持精簡——目標網址、選填的自訂 slug 與送出按鈕——密碼、到期、點擊上限與轉址設定收在與登入表單相同的「詳細設定」折疊區塊中；若開放標題與標籤，則與 slug 並列 |
| FR-PUB-2 | 結果 | 導覽首頁會以資料庫解析 `?shortened=`，並渲染短網址與複製按鈕。查詢字串的內容不會被逐字印出，目的地也永不顯示：同一個頁面不得洩漏他人受密碼保護連結的目標 |
| FR-PUB-3 | 存取層級 | `public_create_access` 為 `guest`（任何人，包含沒有帳號的訪客）、`user`（已登入帳號，預設值與既有行為）或 `admin`。層級為累加關係。此設定把關**每一條**建立路徑——導覽首頁表單、`POST /app/urls` 與 `POST /api/v1/urls`——因此限制為管理員的站台會在各地對成員回 `403`；儀表板則改以提示取代表單 |
| FR-PUB-4 | 訪客所有權 | 匿名訪客建立的連結由保留的訪客帳號擁有（`users.is_system = 1`），因為 `urls.user_id` 是必填外鍵。該帳號由 migration 建立、沒有群組（因此由公開開關而非群組決定訪客能做什麼）、無法登入（`status = suspended`、無法解析的密碼雜湊），也不能編輯或刪除；它不會出現在使用者列表與任何使用者計數中，管理員的網址列表則把它的連結標為 `Guest` |
| FR-PUB-5 | 欄位開關 | 管理員可啟用公開表單提供的選填欄位：自訂 slug 與標題（預設開啟）、標籤、連結密碼、過期、點擊上限、含中轉頁文案的轉址方式，以及網域選擇（預設關閉）。目標網址一律必填。送出時若填了被停用的欄位，會在進行任何工作前以 `403` 拒絕，因此隱藏欄位永遠不是唯一的防線 |
| FR-PUB-6 | 防機器人驗證 | 公開表單由設定的供應商透過專用的 `public_captcha` 開關保護（預設開啟，未設供應商時不生效）。匿名訪客以公開流程驗證；已登入的瀏覽器 Session 則改依 `captcha_on_create`，因此公開端點不能被用來繞過它；`Bearer` API Token 如同其他建立路徑一樣豁免。失敗一律關閉並回應 `?err=captcha` / `400` |
| FR-PUB-7 | 遊客自然週期配額 | `public_rate_limit_per_ip`（預設 20）與 `public_rate_limit_global`（預設 500）**只套用於未登入遊客**。兩者各有 `_period` 設定：`minute`、`hour`（預設）、`day`、`week`、`month`。計數依服務進程時區（`TZ`，未設定則採系統時區）的自然邊界重設：每週從週一 00:00 開始，每月從 1 日 00:00 開始，並正確處理日光節約時間。用量以原子操作存入 SQLite，重啟後保留。所有來源、共用配額與週期合計最多 50,000 列；滿額時新列以 `429` 拒絕，既有列仍保留用量與限制。扣抵前及維護時回收已結束的週期，不會為了新來源淘汰未到期計數。`0` 停用任一限制。登入用戶使用分組的有效連結額度與原有帳號防濫用機制，不消耗遊客配額。單一來源配額在防機器人驗證前扣抵，共用配額只在驗證成功後扣抵。拒絕時回 `429`，`Retry-After` 指向下個自然邊界（表單以 `?err=rate_limited` 轉址） |
| FR-PUB-8 | API | `POST /api/v1/public/urls` 接受與表單相同的內容（外加 `captcha_token`），並回 `201` 與 `domain`、`slug`、`short_url`、`target_url` 與 `created_at`。它刻意不提供連結 id：呼叫者沒有帳號可以管理該連結 |
| FR-PUB-9 | 稽核與配額 | 訪客建立與其他建立一樣都會寫入稽核（`url.created`，actor = 訪客帳號），並計入 `public_max_urls`（預設 1000，`0` = 無限制），也就是訪客帳號可擁有的連結總數。這刻意不是 `max_urls_per_user`：匿名連結的洪水不應耗用管理員為真實帳號設定的配額。配額用滿後，除非刪除那些連結，否則所有訪客都會取得配額錯誤（管理員的網址列表可依擁有者篩選），且維護任務會記錄一筆警告，讓管理者察覺 |
| FR-PUB-10 | 訪客目標網址白名單 | `guest_allowlist_enabled` 預設 `true`，`guest_allowlist` 內建可編輯的 24 個常見服務，涵蓋知識／開發、討論、影音、社群、新聞與支援（43 條主機規則，不限制內建項目的路徑；排除公開網站代管網域）。匿名 HTML／API 建立必須命中精確主機名稱，或以開頭 `*.` 涵蓋所有層級的子網域（不含根網域），以及選填、區分大小寫的路徑前綴；路徑依區段邊界比對，不接受帳密或非標準連接埠。檢查在每來源位址額度之後、驗證碼及共用訪客額度之前。管理設定／API 可編輯最多 200 條規則／16 KiB，立即生效且重啟保留；啟用且清單為空時拒絕所有訪客目標。登入 session／token 與既有連結轉址不受影響。僅補齊缺值的預設設定不覆寫已編輯或刻意清空的清單。首頁顯示允許的目標與專用拒絕訊息。這是本機政策，不是可信度分析或安全保證，不抓取目標內容 |
| FR-PUB-11 | 後台顯示 | 只有允許遊客建立時才顯示遊客速率、週期、連結總額度、驗證與目標白名單設定；切換時即時更新，隱藏後仍保留值。無 JavaScript 時儲存權限後顯示。登入用戶的分組額度說明與共用首頁欄位開關維持可見，計數時區顯示於遊客區塊 |

---

### 4.17 追蹤參數清理 (FR-CLEAN)

| ID | 需求 | 說明 |
|----|------|------|
| FR-CLEAN-1 | 選擇啟用 | `url_cleaning_enabled=false` 為預設。管理員可在 `/app/admin/settings` 或既有設定 API 啟用，套用於網頁表單、登入 API（含 API Token）及公開 API 新建立的連結 |
| FR-CLEAN-2 | 保守內建規則 | 在 Facebook、Instagram、Threads、X/Twitter、YouTube、TikTok 及 Bilibili 移除 `utm_*` 與明確列出的平台追蹤參數。其他網域除非符合自訂規則，否則保持原樣。內建規則保留影片 ID、播放清單與播放時間 |
| FR-CLEAN-3 | 自訂規則 | `url_cleaning_rules` 為預設空白的文字設定。每個非空白、非註解行為 `網域 參數 [參數 ...]`，例如 `example.com utm_* ref`。網域以點號邊界涵蓋子網域；參數名稱解碼後忽略 ASCII 大小寫，尾端 `*` 表示前綴。去除行首空白後以 `#` 開頭者為註解。自訂規則與內建規則合併，清空自訂規則仍保留內建規則 |
| FR-CLEAN-4 | 驗證 | 上限 8192 bytes、100 條規則，每條 32 個參數，每個參數 64 bytes（含 `*`）。網域為 ASCII hostname，至少兩個非空分段，總長最多 253 bytes、每段 63 bytes；每段限英文字母、數字及連字號，且不能以連字號開頭或結尾。不接受 scheme、port、路徑、帳密或萬用網域。參數名稱限英文字母、數字、`_`、`-` 或 `.`，尾端可加一個 `*`，拒絕單獨 `*` 及正規表示式。非法批次不寫入資料庫或快取 |
| FR-CLEAN-5 | 網址保留 | 先驗證原始目標，只移除符合規則的 query 元件（含重複者），保留其餘元件原始編碼、順序、值、路徑及 fragment。移除後 query 為空時不保留問號。不展開分享短鏈、不讀取目標、不改寫網域／路徑，也不在重新導向時清理 |
| FR-CLEAN-6 | 相容性 | 僅建立時清理並儲存／回傳清理後目標。既有連結與明確的目標網址編輯不受後續設定改變影響。停用功能仍保存自訂規則，供日後重新啟用。不新增資料表或 migration，啟動時只補入缺少的設定，不覆寫現值 |
| FR-CLEAN-7 | 管理介面 | 六個主題共用開關、規則文字框、格式／上限／風險提示及可展開的內建規則，支援三語。非法規則送出後回傳 422 頁面，保留經 HTML 跳脫的輸入及本地化錯誤，不儲存任何設定 |

內建規則唯一來源為 `src/rules/url-cleaning.txt`，以與自訂規則相同的格式獨立於 Rust 邏輯維護。透過 `include_str!` 在編譯時嵌入執行檔，清理器與管理介面共用此清單；更新內建規則需修改清單並重新建置，部署時無需額外規則檔。自訂規則可能移除必要參數，因此介面提醒管理員只移除已知追蹤名稱。

---

### 4.18 SNS 分享預覽 (FR-PREVIEW)

| ID | 需求 | 說明 |
|----|------|------|
| FR-PREVIEW-1 | 擁有者設定 | 登入使用者可在連結表單的「SNS 預覽」區塊（建立時或編輯器中，與連結一起以單一 multipart 表單儲存），或透過已認證 API 編輯選填的預覽標題（200 字）、內容（1000 字）與圖片，與管理用標題、備註及中繼頁文字分開。預覽標題留空時使用站台名稱；清空所有欄位即可移除自訂預覽 |
| FR-PREVIEW-2 | 圖片來源 | 支援完整 HTTPS 圖片網址（上限 2048 字，不含帳號密碼或空白），或上傳 PNG、JPEG、GIF、WebP（上限 2 MiB、4096 × 4096 像素，以受限解碼驗證）。上傳圖片存於 SQLite。明確選擇保留、替換或移除；替換／移除後舊圖片網址失效。伺服器不抓取外部圖片或目標網站資料 |
| FR-PREVIEW-3 | 輸出 | 有自訂設定的有效連結向已知 SNS 預覽爬蟲回傳 200 HTML，包含 HTML 跳脫後的 Open Graph／Twitter metadata。一般瀏覽器沿用直接轉址、中繼頁與密碼頁；未設定自訂預覽時行為不變。統計用機器人辨識與 SNS 預覽分類分開；User-Agent 僅決定呈現，不作認證 |
| FR-PREVIEW-4 | 公開與隱私 | 使用者設定的預覽文字與上傳圖片會公開，即使連結有密碼；編輯介面明確提示。預覽 HTML 不輸出目標網址、管理用標題或備註。過期、點擊上限、停用／刪除與無效網域優先於預覽及圖片輸出 |
| FR-PREVIEW-5 | 統計與快取 | 自訂預覽抓取不記錄點擊，不消耗點擊上限。短網址回應包含 Vary: User-Agent，自訂預覽與圖片使用 no-store。第三方平台仍可能保留自己的快取，無法保證卡片版型或即時更新 |
| FR-PREVIEW-6 | 錯誤與權限 | 文字與圖片修改具原子性，限擁有者並遵循既有 CSRF 規則。上傳限制請求大小與速率。無效圖片不覆寫既有設定；重試表單保留文字，檔案依瀏覽器限制需重新選取 |
| FR-PREVIEW-7 | 圖片政策 | 管理員在設定頁或設定 API 選擇 `preview_image_policy=none/url/upload/both`（預設 `both`）。政策套用所有擁有者、表單／API 寫入、metadata 圖片與上傳圖片回應。編輯器隱藏禁止的來源，伺服器同樣拒絕；保留／移除與僅編輯文字仍可使用。不允許的既有圖片會保留但暫停輸出，重新允許後恢復；第三方已複製的外部內容無法撤回 |

---

## 5. 非功能需求

### 5.1 安全性 (NFR-SEC)

| ID | 需求 |
|----|------|
| NFR-SEC-1 | 密碼：Argon2id；Session/API Token 僅存 SHA-256 雜湊，查詢以雜湊值進行索引比對。邀請碼保留 SHA-256 雜湊供查詢，另存以 `URLET_SECRET` 衍生的金鑰進行 ChaCha20-Poly1305 加密封存的副本，讓發碼者與管理員可再次顯示，且資料庫不存明文。但這不是資料庫的機密性邊界：雜湊與四字元前綴仍可離線猜測，且自動產生的 `secret.key` 就在資料庫旁。更換密鑰會讓既有封存無法解開（邀請碼本身仍可兌換）。解鎖 Cookie 的簽章以常數時間比較 |
| NFR-SEC-2 | Cookie：`HttpOnly`、`SameSite=Lax`、`Secure`（當 `URLET_SECURE_COOKIES=true` 或 Base URL 為 https） |
| NFR-SEC-3 | CSRF：狀態變更請求檢查 `Origin`/`Referer` 同源；Cookie 認證 + 非 GET 一律驗證；Bearer Token 請求豁免。外觀偏好端點 `/app/lang/{code}` 與 `/app/theme/*` 為 `GET`，只寫入顯示用 cookie，不會變更資料或權限 |
| NFR-SEC-4 | 安全標頭：`Content-Security-Policy: default-src 'self'; frame-ancestors 'none'`、`X-Content-Type-Options: nosniff`、`Referrer-Policy: strict-origin-when-cross-origin`、`X-Frame-Options: DENY`、https 時 `Strict-Transport-Security` |
| NFR-SEC-5 | 開放轉址防護：目標僅允許 `http` / `https`；拒絕 `javascript:`、`data:`、`file:` 等 |
| NFR-SEC-6 | 保留字防護：`api`、`app`、`assets`、`health`、`static`、`favicon.ico` 等不得作為 Slug |
| NFR-SEC-7 | 錯誤訊息不洩漏內部細節；登入失敗統一回應「Email 或密碼錯誤」 |
| NFR-SEC-8 | 使用者列舉防護：註冊重複 Email 回 409（可接受，屬常見取捨）；登入統一錯誤訊息 |
| NFR-SEC-9 | 速率限制：登入 10 次/5 分鐘/(IP,email) **加** 40 次/5 分鐘/IP、註冊 5 次/小時/IP、解鎖 10 次/5 分鐘/(slug,IP)、建立短網址 120 次/小時/使用者、改密碼 10 次/小時/使用者、匿名建立預設每自然小時每來源 20 次 **加** 全部遊客 500 次，週期可獨立選擇分鐘／時／日／週／月；登入用戶不受遊客配額限制（`0` 會停用任一額度） |
| NFR-SEC-9b | 帳號鎖定：同一 email 15 分鐘內累積 30 次失敗即暫時拒絕登入（回 429），由 `login_attempts` 表持久化，重啟不重置；記憶體速率限制器的 key 以 SHA-256 儲存並有 5 萬筆上限 |
| NFR-SEC-10 | SQL 一律參數化（sqlx bind），無字串拼接（`src/db/` 之外不得出現 SQL） |
| NFR-SEC-11 | 稽核日誌涵蓋所有敏感操作 |
| NFR-SEC-12 | 啟動時若 `URLET_SECRET` 未設定，於資料目錄產生並持久化隨機密鑰（建立時即為 0600；既有檔案若權限過寬會自動收緊） |
| NFR-SEC-13 | 反向代理標頭：`URLET_TRUST_PROXY=true` 時優先採用代理自行改寫的 `X-Real-IP`，其次取 `X-Forwarded-For` **最右側**（最左側可被客戶端偽造） |
| NFR-SEC-14 | 密碼雜湊在 `spawn_blocking` 中執行，並以信號量限制同時最多 4 個 Argon2 計算（每個約 19 MiB） |
| NFR-SEC-15 | 驗證碼驗證失敗一律關閉：以表單編碼 HTTPS POST 搭配 5 秒逾時呼叫設定的供應商，並透過 `ureq` 2 + rustls + webpki-roots 發出（不需 OpenSSL）；逾時、非 2xx 或無法解析的回應一律拒絕請求。secret key 永不回顯於管理後台 |
| NFR-SEC-16 | 語言 cookie `urlet_lang` 刻意不設 HttpOnly（由前端程式讀取），內容僅為白名單語言標籤；切換端點的 `next` 只接受同站相對路徑，以防開放轉址 |
| NFR-SEC-17 | 主題 cookie `urlet_theme` 為 HttpOnly、`SameSite=Lax`、一年，設定時加 `Secure`；管理員專用的預覽 cookie 屬性相同，但生命週期為一小時。其值只會與已知目錄 key 比對；上傳 CSS 由產生的 key 提供，使用 `text/css` 與 `nosniff`，並拒絕外部資源與 `@import`。預覽 cookie 僅對管理員生效，切換端點需要 `theme_switching` 權限（已登入帳號，或 `all` 政策下的未登入訪客），且 `next`／`Referer` 會先化為同站路徑再轉址 |
| NFR-SEC-18 | 公開短網址工具由三層獨立防線守護：`public_create_access` 層級、`public_allow_*` 欄位開關（被停用的欄位一律由伺服器端拒絕）與兩道速率限制，且防機器人驗證預設開啟。表單送出另須通過一般的 CSRF 同源規則，匿名的 JSON API 也無法以無 cookie 的跨站表單觸及；跨站 JSON `POST` 讀不到回應，並受同一組額度限制 |

### 5.2 效能 (NFR-PERF)

| ID | 需求 |
|----|------|
| NFR-PERF-1 | 轉址路徑：以索引查詢 URL 與選填預覽 + 佇列化寫入；目標 P95 < 10 ms（歷史轉址基準 0.74 ms；新增預覽查詢尚未測速） |
| NFR-PERF-2 | SQLite：`journal_mode=WAL`、`synchronous=NORMAL`、`foreign_keys=ON`、`busy_timeout=5000`；每個寫入交易以 `BEGIN IMMEDIATE` 開啟，讓「先讀後寫」的交易從一開始就持有寫鎖，而不會因失去 WAL 快照得到 `SQLITE_BUSY_SNAPSHOT`——該錯誤不會觸發 busy handler，因此 `busy_timeout` 無法涵蓋 |
| NFR-PERF-3 | 連線池：`max_connections=8`；點擊寫入由單一背景 writer 序列化（避免 WAL 因並發寫入無法 checkpoint 而無限成長） |
| NFR-PERF-4 | 列表查詢一律分頁（預設 20，上限 100） |
| NFR-PERF-5 | 統計查詢使用 `url_daily_stats` 與索引，避免全表掃描 |

### 5.3 運維 (NFR-OPS)

| ID | 需求 |
|----|------|
| NFR-OPS-1 | `docker compose up -d` 即完成部署；首次啟動自動遷移 + 建立管理員 |
| NFR-OPS-2 | 健康檢查：`GET /health`（200 JSON）與 `urlet healthcheck` 子命令（供 Docker HEALTHCHECK 使用） |
| NFR-OPS-3 | 資料持久化於 named volume `urlet-data`，路徑 `/data`；上傳樣式表也在此，另有選用的 `urlet-themes` volume 掛載於 `/data/themes` |
| NFR-OPS-4 | 日誌輸出 stdout（`tracing`，JSON 或 compact 由 `URLET_LOG_FORMAT` 控制） |
| NFR-OPS-5 | 設定全部來自環境變數（12-factor），`.env.example` 完整說明 |
| NFR-OPS-6 | 每日維護任務：清理過期 Session、可選清理過期點擊紀錄 |
| NFR-OPS-7 | 備份 = 複製 `urlet.db`（WAL checkpoint 後），以及 `URLET_THEMES_DIR` 的上傳主題檔案；README 與 DEPLOYMENT 有說明 |
| NFR-OPS-8 | 非 root 使用者執行容器；唯讀根檔案系統可運作 |
| NFR-OPS-9 | 語言目錄以 `include_str!` 內嵌於二進位檔，部署仍為單一 binary；唯一可寫入的資產目錄是 `URLET_THEMES_DIR`，供管理員上傳使用 |

### 5.4 可維護性 (NFR-MAINT)

| ID | 需求 |
|----|------|
| NFR-MAINT-1 | 分層：`routes/`（HTTP）→ `services/`（業務）→ `db/`（SQL）；不跨層直接查詢 |
| NFR-MAINT-2 | 單一錯誤型別 `AppError`，集中轉換為 HTTP 回應 |
| NFR-MAINT-3 | 所有 SQL 集中於 `src/db/` 各模組，使用執行期查詢（無需 `DATABASE_URL` 即可編譯） |
| NFR-MAINT-4 | `cargo clippy -- -D warnings` 與 `cargo fmt --check` 通過 |
| NFR-MAINT-5 | 單元測試 + 整合測試（含 `tower::ServiceExt::oneshot` 端到端）覆蓋核心流程 |
| NFR-MAINT-6 | 所有介面字串只存在於三份語言目錄中；模板與 handler 不得硬寫顯示文字 |

---

## 6. 資料模型

### 6.1 ER 概觀

```
user_groups ─┬─< users
             ├─< invite_codes
             └─< group_domains >─ domains
users ─┬─< sessions
       ├─< api_tokens
       ├─< urls ─┬─< clicks
       │         ├─< url_daily_stats
       │         ├─< url_tags >─ tags
       │         └─< url_previews（選填，每條連結至多一筆）
       ├─< tags
       ├─< invite_redemptions >─ invite_codes
       └─< audit_logs (actor)
domains ─< urls
settings（key-value）
public_creation_quota（遊客自然週期計數）
themes（獨立）
custom_themes（中介資料；CSS 在持久化 volume）
login_attempts（稽核用）
```

### 6.2 設計原則

1. **時間欄位**：全部 `TEXT` 存 ISO-8601 UTC（`YYYY-MM-DDTHH:MM:SS.sssZ`），可直接字串比較與排序。
2. **主鍵**：`INTEGER PRIMARY KEY AUTOINCREMENT`（SQLite rowid 最佳化）。
3. **軟刪除**：`users.deleted_at`、`urls.deleted_at`；查詢一律加 `deleted_at IS NULL`。
4. **枚舉**：以 `TEXT + CHECK` 約束，保留未來擴充彈性（SQLite 無原生 ENUM）。
5. **布林**：`INTEGER 0/1`。
6. **彙總表**：`url_daily_stats` 為統計查詢的加速層，寫入時 upsert。
7. **保留擴充欄位**：`api_tokens.scopes`、`urls.notes`、`audit_logs.metadata`。
8. **群組繼承**：`user_groups.max_urls` 與 `max_api_tokens` 可為 `NULL`；`NULL` 代表跟隨全站設定，`0` 代表無上限。功能開關為 `INTEGER 0/1`，預設 `1`（允許），因此群組在管理員編輯前不會改變任何行為。
9. **參照回退**：`users.group_id` 與 `invite_codes.group_id` 可為空並使用 `ON DELETE SET NULL`；未分組使用者或未指定群組的邀請碼會回退到預設群組或全站預設。

> 多網域支援已實作；完整設計、路由矩陣與 migration 說明見 [`docs/zh-TW/MULTI_DOMAIN.md`](MULTI_DOMAIN.md)。

### 6.3 DDL（migration 摘要）

> 完整真實來源為 `migrations/` 目錄（`0001_init.sql` 至 `0017_public_creation_quota.sql`）：`0003_multi_domain.sql` 重建網域 schema，`0004_user_groups.sql`–`0007_member_invites.sql` 新增群組與邀請碼配額，`0010`／`0012`／`0013` 新增主題，`0011_splash_pages.sql` 新增轉址中繼頁，`0014_invite_issuance_released.sql` 新增邀請碼退還欄位，`0015_public_shorten.sql` 新增 `users.is_system`、保留的訪客帳號與公開短網址工具設定；`0016_url_previews.sql` 新增 SNS 預覽資料與圖片（每條連結至多一筆）；`0017_public_creation_quota.sql` 新增遊客自然週期計數與週期設定；以下為結構摘要。

| 表 | 主要欄位 | 用途 |
|----|----------|------|
| `users` | id, email, email_normalized(UNIQUE), password_hash, display_name, role, status, group_id, is_system, email_verified_at, last_login_at, created_at, updated_at, deleted_at | 使用者；`is_system = 1` 標記擁有匿名連結的保留訪客帳號 |
| `user_groups` | id, name(UNIQUE, NOCASE), description, max_urls, max_api_tokens, max_invites, invite_period, allow_invites, allow_custom_slug, allow_password, allow_expiry, allow_max_clicks, allow_tags, allow_api_tokens, allow_domain_selection, redirect_mode_policy, default_domain_id, restrict_domains, is_default, created_at, updated_at | 配額、功能開關、邀請碼、轉址方式與網域策略群組 |
| `group_domains` | group_id, domain_id (PK) | 各群組的明確網域允許清單 |
| `sessions` | id, user_id, token_hash(UNIQUE), user_agent, ip_hash, created_at, last_seen_at, expires_at, revoked_at | 登入狀態 |
| `api_tokens` | id, user_id, name, token_prefix, token_hash(UNIQUE), scopes, last_used_at, expires_at, created_at, revoked_at | API 憑證 |
| `domains` | id, hostname(UNIQUE, NOCASE), is_default, is_verified, status, source, created_by, created_at, updated_at | Canonical/system、`URLET_DOMAINS`/env 與管理員/UI 網域；`pending`/`active`/`disabled` |
| `urls` | id, user_id, domain_id(NOT NULL), slug, target_url, title, notes, password_hash, expires_at, max_clicks, click_count, unique_click_count, redirect_type, redirect_mode, splash_delay_seconds, splash_title, splash_message, status, is_custom_slug, last_clicked_at, created_at, updated_at, deleted_at, UNIQUE(domain_id,slug) | 短網址；slug 命名空間以網域為單位 |
| `tags` | id, user_id, name, color, created_at, UNIQUE(user_id,name) | 標籤 |
| `url_tags` | url_id, tag_id (PK) | 標籤關聯 |
| `clicks` | id, url_id, clicked_at, ip_hash, visitor_hash, user_agent, referrer, referrer_host, country_code, device_type, browser, os, is_bot, is_unique, password_protected | 點擊明細 |
| `url_daily_stats` | url_id, day, clicks, unique_clicks, bot_clicks (PK: url_id+day) | 每日彙總 |
| `audit_logs` | id, actor_user_id, actor_role, action, target_type, target_id, metadata, ip_hash, created_at | 稽核 |
| `themes` | id, key(UNIQUE), is_enabled, is_default, sort_order, created_at, updated_at | 管理員對所有主題的選擇；上傳主題註冊時會建立對應資料列；以 partial unique index 確保唯一預設 |
| `custom_themes` | id, key(UNIQUE), name, description, appearance, is_dark_only（舊版）, css_version, css_size, original_filename, created_by, created_at, updated_at | 管理員上傳樣式表的中介資料，含 `auto`／`light`／`dark` 外觀；檔案位於 `URLET_THEMES_DIR` |
| `invite_issuance` | id, group_id, actor_id, invite_id, created_at, released_at | 各群組邀請碼發行帳本；未被兌換即撤銷的資料列（`released_at`）不再計入配額 |
| `url_previews` | url_id(PK, FK), title, description, image_url, image_key(UNIQUE), image_mime, image_data | 選填的公開 SNS 卡片與上傳圖片；metadata 查詢不載入 BLOB |
| `public_creation_quota` | scope(雜湊), period, window_start, window_end, used (PK: scope+period+window_start) | 遊客自然週期用量；重啟保留並原子扣抵 |
| `settings` | key(PK), value, updated_at, updated_by | 全站設定 |
| `invite_codes` | id, code_hash(UNIQUE), code_prefix, code_sealed, note, group_id, created_by, max_uses, used_count, expires_at, status, created_at | 邀請碼；`code_sealed` 為可再次顯示邀請碼的加密封存 |
| `invite_redemptions` | id, invite_id, user_id, redeemed_at, UNIQUE(invite_id,user_id) | 邀請碼使用紀錄 |
| `login_attempts` | id, identifier_hash, ip_hash, success, created_at | 登入稽核/鎖定 |

### 6.4 索引策略

| 索引 | 目的 |
|------|------|
| `urls(domain_id, slug)` UNIQUE | 轉址查詢（最熱路徑；slug 以網域為命名空間） |
| `urls(user_id, created_at DESC)` | 我的網址列表 |
| `urls(expires_at) WHERE expires_at IS NOT NULL` | 過期清理/篩選 |
| `clicks(url_id, clicked_at DESC)` | 單一網址統計 |
| `clicks(clicked_at)` | 保留期清理 |
| `sessions(user_id, expires_at)`、`sessions(expires_at)` | Session 驗證與清理 |
| `api_tokens(token_hash)` UNIQUE | Token 認證 |
| `url_daily_stats(day)` | 全站趨勢 |
| `audit_logs(created_at DESC)`、`audit_logs(actor_user_id, created_at DESC)`、`audit_logs(target_type,target_id)` | 稽核查詢 |
| `login_attempts(identifier_hash, created_at DESC)` | 暴力破解偵測 |
| `user_groups(is_default) WHERE is_default = 1` UNIQUE | 最多一個預設群組 |
| `users(group_id)` | 群組成員列表與改派 |
| `invite_codes(group_id)` | 依群組查詢邀請碼 |
| `group_domains(domain_id)` | 網域刪除與允許清單查詢 |
| `themes(is_default) WHERE is_default = 1` UNIQUE | 最多一個預設主題 |
| `custom_themes(created_by)` | 上傳歸屬與清理 |
| `users(is_system) WHERE is_system = 1` UNIQUE | 最多一個保留的系統帳號 |

v1.1 的變更只新增 `settings` 的 key；由於 `settings` 為 key-value 表，v1.1 維持 **14 張表、27 個索引**。
v1.2 新增 `user_groups` 與 `group_domains` 表、可空的 `users.group_id` / `invite_codes.group_id` 欄位、
`user_groups.default_domain_id` / `restrict_domains` 欄位及四個索引，共 **16 張表、31 個索引**（§6.1、§6.3、§6.4）。
邀請碼配額工作新增 `invite_issuance` 表與索引，使 schema 成為 **17 張表、33 個索引**；v1.3 僅新增可空的
`invite_codes.code_sealed` 欄位，表與索引數量不變；v1.4 新增 `themes` 表及其唯一預設索引。
v1.5 新增 `urls` 的中轉頁欄位與 `user_groups.redirect_mode_policy`（表／索引數量不變），
可上傳主題再新增 `custom_themes` 與其建立者索引，共 **19 張表、35 個索引**。
邀請碼退還額度工作新增可空的 `invite_issuance.released_at` 欄位，表／索引數量維持 19/35。
升級後首次啟動時，也會釋放「已撤銷且從未兌換」的既有邀請碼，因此 `invites_used` 可能一次下降。
v1.6 新增 `users.is_system` 欄位與其部分唯一索引，共 **19 張表、36 個索引**；SNS 預覽新增 `url_previews` 且不增加明確定義的索引；自然週期配額再新增 `public_creation_quota` 與到期索引，目前為 **21 張表、37 個索引**。

`url_previews` 的 metadata 查詢不載入圖片 BLOB，備份仍以資料庫為單位。Trigger 會在所屬連結被軟刪除時移除預覽資料與圖片（包含帳號刪除）；停用或過期則保留預覽。SQLite 可重用釋放的頁面，資料庫檔案不會自動縮小。

### 6.5 預設資料 (seed)

- SNS 圖片：`preview_image_policy=both`；允許值為 `none`、`url`、`upload`、`both`。
- 網址清理：`url_cleaning_enabled=false`、`url_cleaning_rules=`（空白）；啟動時只補入缺少的鍵並保留現值。

- `settings`：`site_name=urlet`、`registration_mode=open`、`default_slug_length=7`、
  `max_urls_per_user=1000`、`default_redirect_type=302`、`analytics_enabled=true`、`click_retention_days=0`。
- `settings`（v1.1 驗證碼 key）：`captcha_provider=none`、`captcha_site_key=`（空）、
  `captcha_secret_key=`（空）、`captcha_on_register=false`、`captcha_on_login=false`、
  `captcha_on_create=false`、`captcha_allowed_hostnames=`（空）。非空的環境變數會覆蓋表中值（FR-BOT-2）。
- `settings`（v1.2 新 key）：`max_api_tokens_per_user=20`（全站 Token 配額；`0` = 無上限）。
  由於預設為 20 而非 0，升級後若帳號已持有超過 20 個可用 Token，必須先撤銷部分 Token 才能再新增；
  升級不會主動撤銷任何既有 Token。
- `settings`（v1.2 邀請碼配額，`0006_group_invite_quotas.sql`）：`max_invites_per_period=0`
  （`0` = 無上限）、`invite_period=month`（`day`、`week` 或 `month`；未綁定群組的邀請碼所使用的
  自然週期）。
- `settings`（v1.4 新 key）：`theme_switching=all`（誰能切換主題；`admin` 僅限管理員）。
- `user_groups`（v1.2）：建立一筆名為 `Standard`、`max_urls = NULL`、`max_api_tokens = NULL`、
  七個開關皆為 `1`（含 `allow_domain_selection`）、`default_domain_id = NULL`、
  `restrict_domains = 0`、`is_default = 1` 的群組；遷移會把所有既有使用者指派進去。
- 中轉頁預設值（v1.5）：欄位預設值讓既有連結成為 `redirect_mode = direct`、`splash_delay_seconds = 5`
  且沒有自訂文案，既有群組則為 `redirect_mode_policy = choice`，因此升級後行為不變。
- `group_domains`：不種任何資料；沒有資料列的既有或新群組可使用所有 active 網域。
- 公開短網址工具（v1.6）：`public_create_access=user`（升級後維持原有行為）、
  `public_captcha=true`、`public_rate_limit_per_ip=20`、`public_rate_limit_per_ip_period=hour`、
  `public_rate_limit_global=500`、`public_rate_limit_global_period=hour`、
  `public_max_urls=1000`、
  `public_allow_slug=true`、`public_allow_title=true`，而 `public_allow_tags`、
  `public_allow_password`、`public_allow_expiry`、`public_allow_max_clicks`、
  `public_allow_redirect`、`public_allow_domain` 皆為 `false`；`guest_allowlist_enabled=true`，
  `guest_allowlist` 以內建的 43 條主機規則清單（`src/services/guest_allowlist.txt`，由
  `src/db/settings.rs` 寫入）作為預設，僅在缺少時插入，因此已編輯或刻意清空的清單不會被覆寫。
  migration 也會插入保留的
  訪客帳號（`guest@urlet.invalid`、`is_system = 1`、`status = suspended`、無群組）；
  只有在該位址已屬於其他人的極端情況下才會略過，並於啟動時回報。
- `domains`：啟動時依 `URLET_BASE_URL` 的 host 插入/更新 canonical 網域（`source=system`，永遠 active）。
  只有在沒有任何預設網域時才會設為預設，讓管理員可以改選其他預設。`URLET_DOMAINS` 只插入缺少的
  網域（`active`/`env`），不改既有資料。
- `settings`（首頁時鐘）：`landing_clock=server`（`server`、`utc` 或 `off`）。它決定公開首頁底部資訊條
  要顯示什麼——行程時區（`TZ`）的時鐘、固定為 UTC 的同一時鐘，或完全不顯示時鐘——因為時區名是粗略的
  位置線索。不論哪一種，版本與運行時間都會保留；訪客配額的週期一律跟隨 `TZ`，所以此設定不會改變實例
  對「本地時間」的定義。

---

## 7. API 規格

### 7.1 認證

| 方法 | 路徑 | 說明 |
|------|------|------|
| POST | `/api/v1/auth/register` | `{email, password, invite_code?, captcha_token?}` → 201 `{user}`；驗證碼驗證失敗回 400 `captcha_failed` |
| POST | `/api/v1/auth/login` | `{email, password, captcha_token?}` → 200 `{user}` + Set-Cookie；驗證碼驗證失敗回 400 `captcha_failed` |
| POST | `/api/v1/auth/logout` | 撤銷當前 Session → 204 |
| GET | `/api/v1/auth/me` | 當前使用者 → 200 `{user}`；另含 `group`（id/name 或 null）與 `limits`（生效配額、開關、轉址方式政策、預設網域與允許網域，FR-GROUP-8） |
| PATCH | `/api/v1/auth/me` | `{display_name}` → 200 `{user}` |
| POST | `/api/v1/auth/password` | `{current_password, new_password}` → 204 |
| GET | `/api/v1/auth/sessions` | 我的 Session 清單 |
| DELETE | `/api/v1/auth/sessions/{id}` | 撤銷指定 Session → 204 |

註冊時指派預設群組；即使在 `open` 模式，提供的邀請碼也會驗證，且其群組優先於預設群組
（FR-GROUP-4）。網頁表單可用 `GET /app/register?invite=CODE` 預填邀請碼；JSON 內容不變。
`user` 物件包含 `group_id`；群組名稱與生效額度透過 `GET /api/v1/auth/me` 取得。

### 7.2 短網址

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/urls` | 查詢參數：`q, tag, status, page, per_page, sort(created_at\|clicks(_desc)\|slug(_asc)\|updated(_desc)\|expires(_asc))` |
| POST | `/api/v1/urls` | `{target_url, domain?, slug?, title?, notes?, tags?, password?, expires_at?, max_clicks?, redirect_type?, redirect_mode?, splash_delay_seconds?, splash_title?, splash_message?, captcha_token?}`；API Token 呼叫豁免驗證碼，Session Cookie 呼叫受 `on_create` 約束 |
| GET | `/api/v1/domains` | 建立短網址時可選的 active 網域，已依呼叫者的群組策略過濾（含生效的 `is_default`） |
| GET | `/api/v1/urls/{id}` | 單筆（含 `short_url`、`is_expired`、`has_password`） |
| PATCH | `/api/v1/urls/{id}` | 部分更新；`domain` 可搬移連結；`password: null` 清除密碼；`expires_at: null` 清除過期；`splash_title` / `splash_message: null` 清除自訂文案 |
| DELETE | `/api/v1/urls/{id}` | 軟刪除 → 204 |
| GET | `/api/v1/urls/{id}/preview` | 該連結的 SNS 預覽：`{title, description, image_url, image_uploaded, enabled}` |
| PATCH | `/api/v1/urls/{id}/preview` | `{title?, description?, image_url?}`；`null` 清除單一欄位，全部清空即移除自訂預覽；被 `preview_image_policy` 禁止的來源會被拒絕 |
| DELETE | `/api/v1/urls/{id}/preview` | 移除自訂預覽與其上傳圖片 → 204 |
| POST | `/api/v1/urls/{id}/preview/image` | multipart `image` 上傳（PNG/JPEG/GIF/WebP，≤ 2 MiB，≤ 4096 × 4096）→ 預覽 |
| GET | `/api/v1/urls/{id}/stats?days=30` | 每日趨勢 + 維度分佈 + 摘要 |
| GET | `/api/v1/urls/{id}/clicks?page=1` | 點擊明細（分頁） |

連結配額用盡、群組開關封鎖了送入的欄位，或轉址方式不符合群組政策時，回 403 `forbidden` 並附說明訊息。
`PATCH` 時省略欄位代表維持原值，清除被封鎖的欄位則可接受；新開啟被封鎖的功能會被拒絕。
非空的連結密碼一律視為新值，因此在密碼被封鎖時會被拒絕（FR-GROUP-3）。
轉址方式政策會拒絕新的 `redirect_mode`（或 `PATCH` 時切換到被封鎖的模式），但允許維持目前模式與清除中轉頁文案。

### 7.3 標籤 / Token

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/tags` | 我的標籤（含使用次數） |
| DELETE | `/api/v1/tags/{id}` | 刪除標籤（僅解除關聯） |
| GET | `/api/v1/tokens` | 我的 API Token 清單（不含明文） |
| POST | `/api/v1/tokens` | `{name, expires_at?}` → 201 `{token: "ur_...", api_token}` **僅此一次**；群組禁止建立 Token 或配額用盡時回 403 |
| DELETE | `/api/v1/tokens/{id}` | 撤銷 → 204 |

生效 Token 配額只計算可用 Token（未撤銷且未過期），因此撤銷舊 Token 會立即釋出一個名額（FR-TOK-6）。

### 7.4 成員邀請碼

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/invites?page=&per_page=` | 呼叫者自己的邀請碼，新的在前，含 `code`、`link`、`prefix`、`group_id`、`max_uses`、`used_count`、`status` 與生效的 `quota` |
| POST | `/api/v1/invites` | `{note?, max_uses?}` → 201 `{code, link, invite, quota}`；邀請碼固定綁定呼叫者的群組；群組停用邀請或週期配額用盡回 `403`，每小時突發限制用盡回 `429` |
| DELETE | `/api/v1/invites/{id}` | 撤銷呼叫者自己的邀請碼 → 204；他人的邀請碼回 404 |

成員邀請碼需要 `user_groups.allow_invites = 1`；邀請碼一律綁定呼叫者的群組，因此兌換者會加入該
群組，管理員與成員發行的邀請碼共用同一份各群組帳本（`max_invites` / `invite_period`，migrations
`0006`/`0007`）。成員邀請碼的 `max_uses` 為 1–10。

### 7.5 管理（需 admin）

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/admin/users` | 使用者列表（`q, role, status, group_id, page`） |
| PATCH | `/api/v1/admin/users/{id}` | `{role?, status?, display_name?, group_id?}`；`group_id: null` 表示改為未分組（套用全站預設） |
| POST | `/api/v1/admin/users/{id}/password` | `{new_password}` 重設 |
| DELETE | `/api/v1/admin/users/{id}` | 軟刪除（其網址一併軟刪除） |
| GET | `/api/v1/admin/domains` | 所有網域，含連結數、狀態與來源 |
| POST | `/api/v1/admin/domains` | `{hostname}` → 201，建立為 `pending` |
| PATCH | `/api/v1/admin/domains/{id}` | `{status?: "active"\|"disabled", is_default?: true}` |
| DELETE | `/api/v1/admin/domains/{id}` | 204；僅限零連結、非預設的 UI 網域 |
| GET | `/api/v1/admin/groups` | 使用者群組列表（含成員數、儲存的預設/允許網域欄位，分頁） |
| POST | `/api/v1/admin/groups` | `{name, description?, max_urls?, max_api_tokens?, allow_*?, redirect_mode_policy?, default_domain_id?, restrict_domains?, domain_ids?, is_default?}` → 201；domain_ids 必須存在，default_domain_id 必須 active；名稱重複或第二個預設群組回 409 |
| PUT | `/api/v1/admin/groups/{id}` | `name` 為必填；其餘省略的欄位維持原值；`max_urls` / `max_api_tokens` 設為 `null` 表示跟隨全站配額；`description: null` 會清除說明；`default_domain_id: null` 表示全站預設；`domain_ids: null` 或 `[]` 會清空允許清單；`is_default: true` 會在同一交易內轉移預設標記 |
| DELETE | `/api/v1/admin/groups/{id}?reassign_to={id}` | 先把成員搬到 `reassign_to`，再刪除 → 204；預設群組或目標不存在/不合法回 409 |
| GET | `/api/v1/admin/themes` | 主題目錄：key、名稱、說明、`enabled`、`default`、`appearance`、`dark_only`、`light_only`、`css_url`、`custom`，以及目前的 `switching` 政策 |
| PUT | `/api/v1/admin/themes` | `{enabled?: [keys], default?: key, switching?: "all"\|"admin"}`；省略的欄位維持原值；至少一個啟用主題且預設必須啟用。`keys` 可包含 `custom-…` 上傳主題 |
| GET | `/api/v1/admin/urls` | 全站網址列表 |
| PATCH | `/api/v1/admin/urls/{id}` | 強制停用/啟用/刪除 |
| GET | `/api/v1/admin/invites` | 邀請碼列表（新的在前），含 `code`、`link`、`prefix`、`group_id` / `group_name`；最多回傳最新的 200 筆邀請碼，且沒有分頁。舊版或密鑰更換後的列為 `null`；已撤銷、次數用盡或到期的邀請碼 `link` 也為 `null` |
| POST | `/api/v1/admin/invites` | `{max_uses?, expires_at?, note?, group_id?}` → 201 `{code: "K7M2QPX4RT", link: "https://host/app/register?invite=K7M2QPX4RT", invite}`；`link` 與 `invite.link` 一致，若建立後已不可用則為 `null`；`GET` 一律保留 `code`，`link` 只在可用時回傳 |
| DELETE | `/api/v1/admin/invites/{id}` | 撤銷 → 204 |
| GET | `/api/v1/admin/settings` | 全站設定 |
| PUT | `/api/v1/admin/settings` | 更新設定（白名單鍵，含 `max_api_tokens_per_user`） |
| GET | `/api/v1/admin/audit` | 稽核日誌（`action, target_type, actor, page`） |
| GET | `/api/v1/admin/stats` | 系統資訊與總量 |

群組更新接受部分表示：省略欄位即維持原值，`max_urls` / `max_api_tokens` 設為 `null` 代表
跟隨全站配額，`description: null` 則清除說明。`POST` 仍採建立預設值（省略開關視為允許、
省略配額視為跟隨全站）。所有群組異動都會寫入稽核（`admin.group_created`、
`admin.group_updated`、`admin.group_deleted`），使用者的群組改派則記錄在既有的
`admin.user_updated` metadata 中。

### 7.6 公開

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/` | 導覽首頁（HTML）；當訪客可以建立連結時，另含公開短網址工具 |
| POST | `/` | 公開短網址工具表單（HTML）。成功時 303 → `/?shortened=<slug>&domain=<hostname>`；失敗時 303 → `/?err=<code>`，為 `denied`（低於存取層級，或填了被停用的欄位）、`rate_limited`、`captcha`、`quota`、`conflict`、`slug`、`target`、`expiry`、`tags`、`password_rules`、`domain`、`mode_denied` 或 `invalid` 之一。僅限 canonical 主機 |
| POST | `/api/v1/public/urls` | 公開短網址工具（JSON）：`{target_url, domain?, slug?, title?, tags?, password?, expires_at?, max_clicks?, redirect_mode?, splash_delay_seconds?, splash_title?, splash_message?, captcha_token?}` → `201 {domain, slug, short_url, target_url, created_at}`。低於存取層級或填了被停用的欄位回 `403`，防機器人驗證失敗回 `400`，額度用盡回 `429` |
| GET | `/health` | `{status, version, database, uptime_seconds}` |
| GET | `/app/lang/{code}` | 設定 `urlet_lang` cookie 並 302 至 `next`；`next` 未帶或不是同站相對路徑時轉往 `/app` |
| GET | `/{slug}` | 轉址（302 等）、中轉頁（HTML）或密碼頁（HTML）或 410/404 頁 |
| POST | `/{slug}/unlock` | 表單或 JSON `{password}`；成功時 JSON 回 200 `{ok, redirect}`、表單 303 → `/{slug}` |
| GET | `/api/v1/public/urls/{slug}?domain=<hostname>` | 公開中介資料：`{domain, slug, has_password, redirect_mode, is_expired, state, title?}`（**不含 target**）；省略 `domain` 時使用預設網域 |

---

## 8. 設定（環境變數）

| 變數 | 預設 | 說明 |
|------|------|------|
| `TZ` | 系統時區；Docker 預設 `UTC` | 遊客自然週期配額使用的服務進程時區，例如 `Asia/Taipei`；啟動時讀取 |
| `URLET_BIND` | `0.0.0.0:8080` | 監聽位址 |
| `URLET_BASE_URL` | `http://localhost:8080` | Canonical 對外網址（用於產生短網址、CSRF 同源檢查、Cookie Secure 判斷） |
| `URLET_DOMAINS` | 空 | 額外網域（僅轉址），逗號分隔；第一次啟動插入為 `active`，不覆寫既有資料 |
| `URLET_DATABASE_URL` | `sqlite:///data/urlet.db?mode=rwc` | SQLite 連線字串 |
| `URLET_DATA_DIR` | 資料庫所在目錄 | 自動產生密鑰的存放位置 |
| `URLET_THEMES_DIR` | `<URLET_DATA_DIR>/themes` | 上傳主題樣式表的存放目錄；docker compose 將 `urlet-themes` volume 掛載於此 |
| `URLET_HEALTHCHECK_URL` | 由 `URLET_BIND` 推導 | `urlet healthcheck` 的目標 URL |
| `URLET_SECRET` | 自動產生並存檔 | Session/解鎖 Cookie/IP 雜湊的 HMAC 密鑰（至少 16 字元；自動產生的為 44 字元 base64，寫入資料目錄的 `secret.key`） |
| `URLET_ADMIN_EMAIL` | — | 啟動時建立/升級的管理員 Email |
| `URLET_ADMIN_PASSWORD` | — | 上述管理員密碼（僅在帳號不存在時使用） |
| `URLET_REGISTRATION` | 讀取設定表 | 覆寫註冊模式 `open\|invite\|closed` |
| `URLET_SECURE_COOKIES` | 依 `BASE_URL` 判斷 | 強制 Cookie `Secure` |
| `URLET_TRUST_PROXY` | `false` | 信任 `X-Forwarded-For` / `X-Forwarded-Proto` |
| `URLET_SESSION_TTL_HOURS` | `168` | Session 有效期（小時） |
| `URLET_UNLOCK_TTL_MINUTES` | `30` | 短網址解鎖 Cookie 有效期（分鐘） |
| `URLET_LOG` | `info` | `tracing` 過濾器 |
| `URLET_LOG_FORMAT` | `compact` | `compact` \| `json` |
| `URLET_DEFAULT_SLUG_LENGTH` | 讀取設定表 | 覆寫自動 Slug 長度（4–16） |
| `URLET_CAPTCHA_PROVIDER` | `none` | 防機器人驗證供應商：`none` \| `turnstile` \| `recaptcha`；非空時覆蓋 `captcha_provider` |
| `URLET_CAPTCHA_SITE_KEY` | — | 供應商 site key；非空時覆蓋 `captcha_site_key` |
| `URLET_CAPTCHA_SECRET_KEY` | — | 供應商 secret key；非空時覆蓋 `captcha_secret_key` |
| `URLET_CAPTCHA_ON_REGISTER` | `false` | 註冊表單是否要求驗證碼；非空時覆蓋 `captcha_on_register` |
| `URLET_CAPTCHA_ON_LOGIN` | `false` | 網頁登入表單與 `POST /api/v1/auth/login` 是否要求驗證碼；非空時覆蓋 `captcha_on_login` |
| `URLET_CAPTCHA_ON_CREATE` | `false` | 網頁建立短網址表單是否要求驗證碼；非空時覆蓋 `captcha_on_create` |
| `URLET_CAPTCHA_ALLOWED_HOSTNAMES` | — | 供應商 `hostname` 回應中額外接受的主機名稱，以逗號分隔（最多 10 項）；非空時覆蓋 `captcha_allowed_hostnames`。`URLET_BASE_URL` 的主機名稱一律接受 |

驗證碼相關變數的覆蓋規則與 `URLET_REGISTRATION`、`URLET_DEFAULT_SLUG_LENGTH` 相同：
非空的環境變數優先於 `settings` 表中的值（FR-BOT-2）。

---

## 9. 部署

### 9.1 目錄結構

原始碼樹：見 [`README.zh-TW.md`](../../README.zh-TW.md) 的「專案結構」一節，列出 `src/`（模組、
middleware 與 routes）、`templates/`、`migrations/`、`scripts/` 與測試樹。執行期 `/data` 配置
——`urlet.db` 及其 WAL/SHM 檔、自動產生的 `secret.key`，以及 `themes/` 下的上傳主題樣式表
——見 [`DEPLOYMENT.md`](DEPLOYMENT.md) 的「資料與備份」一節。Migration 檔名列於 §6.3。

### 9.2 Dockerfile 策略

多階段建置：
1. `rust:1-alpine` + `build-base` → `cargo build --release --target x86_64-unknown-linux-musl`（LTO、`strip`、`opt-level=z`）
2. `alpine:3.21` → 複製二進位 + `ca-certificates` + `tzdata`，建立非 root 使用者，`HEALTHCHECK CMD ["/usr/local/bin/urlet","healthcheck"]`

### 9.3 Compose 服務

`docker-compose.yml` 執行單一 `urlet` 容器，預設從本機 `Dockerfile` 建置（也可透過
`docker-compose.image.yml` 使用已發布的映像），並使用 `urlet-data` 與 `urlet-themes` 兩個
volume。設定以容器環境變數傳入，支援 `${VAR:-default}` 替換；主機連接埠預設為 `8080`；容器
健康檢查呼叫 `urlet healthcheck`。實際映像標籤、volume 掛載與重啟政策以
[`docker-compose.yml`](../../docker-compose.yml) 為準。

`docker-compose.tls.yml`：額外加入 Caddy（80/443、自動 Let's Encrypt），
urlet 不對外暴露連接埠，由 Caddy 反向代理。

---

## 10. 測試策略

| 層級 | 範圍 | 工具 |
|------|------|------|
| 單元測試 | slug 產生/驗證/保留字、UA 解析、過期判定、設定解析、群組配額繼承（NULL／0／自訂值、未分組回退）、密碼雜湊、IP 雜湊、CSRF 同源判斷、`Accept-Language` 協商（`q=` 權重、回退英文）、語言目錄回退、驗證碼啟用條件矩陣、驗證碼失敗關閉判定 | `#[cfg(test)]` 模組 |
| 整合測試 | 註冊→登入→建立→轉址→統計；密碼保護流程；過期與點擊上限；所有權隔離；admin 授權；API Token；群組 CRUD 與預設標記轉移；各群組的連結與 Token 配額；建立與更新時的功能開關強制；帶群組的邀請碼；刪除群組並改派；CSRF 拒絕；語言協商與 cookie 切換；API JSON 維持英文；註冊、登入與建立時的驗證碼拒絕（網頁與 JSON API）；導覽首頁；中轉頁渲染、密碼優先與轉址方式政策 | `tests/`——`auth.rs`、`urls.rs`、`redirect.rs`、`splash.rs`、`pages.rs`、`admin.rs`、`groups.rs`、`invites.rs`、`i18n.rs`、`captcha.rs`、`multi_domain.rs`、`previews.rs`、`public_urls.rs`、`themes.rs`、`url_cleaning.rs`、`guest_allowlist.rs`，外加 `contract.rs`、`api_docs.rs` 與 `ci_workflow.rs`——以 `tower::ServiceExt::oneshot` 對臨時 SQLite 執行。`contract.rs` 檢查 §4.9 所述的 router 與合約集合相等，`ci_workflow.rs` 檢查每個 CI job 都在 gate 的 `REQUIRED` 清單中 |
| 端對端煙霧測試 | 以真實二進位 + curl 走完主要流程 | `scripts/smoke.sh` |
| 靜態檢查 | `cargo fmt --check`、`cargo clippy -D warnings` | CI 指令（README 說明） |

---

## 11. 驗收標準 (Definition of Done)

v1.0：

- [x] `cargo build --release` 成功，二進位 **4.2 MB** ≤ 15 MB
- [x] `cargo test` 全綠（單元 + 整合 + 二進位）
- [x] `cargo clippy --all-targets -- -D warnings` 與 `cargo fmt --check` 無警告
- [x] `scripts/smoke.sh` 對真實二進位全數通過（含密碼保護、過期、所有權隔離、CSRF、邀請碼、重啟持久化）
- [x] `docker compose config` 通過（base 與 TLS overlay 皆以 Docker Compose 27.4 驗證）
- [x] `docker build` 成功，映像 **13.2 MB**；`docker compose up -d` 後容器狀態 `healthy`
- [x] 容器內以 uid 10001 執行、根檔案系統唯讀、`cap_drop: ALL`；資料僅寫入 `/data` volume
- [x] 容器實測：health、管理員登入、建立含密碼/過期/點擊上限的短網址、解鎖轉址、全部管理頁面 200、重啟後 Session 與解鎖 Cookie 仍有效
- [x] 14 張表 + 索引依 §6 建立，啟動自動遷移（`schema_migrations` 有 checksum 保護）
- [x] 所有 FR/NFR 條目皆有對應實作或測試
- [x] README 可讓新使用者在 5 分鐘內完成部署
- [x] 每個階段皆有對應的本地 commit（見 `docs/PLAN.md`）

v1.1 新增項目：

- [x] 三份語言目錄（`en`、`zh-Hant`、`zh-Hans`）內嵌於二進位檔並涵蓋所有介面字串，缺少 key 時回退英文
- [x] `Accept-Language` 協商正確處理 `q=` 權重，無法辨識或未帶標頭時預設英文
- [x] `GET /app/lang/{code}?next=<path>` 設定 `urlet_lang`（Path=/、SameSite=Lax、Max-Age 一年、非 HttpOnly）並回 302 轉往 `next` 或 `/app`，且 `next` 只接受同站相對路徑
- [x] `<html lang>` 反映當前語言，錯誤頁面亦依語言呈現
- [x] HTML 回應帶 `Content-Language` 與 `Vary: Accept-Language, Cookie`；靜態資源不設 `Vary`
- [x] JSON API 回應不受請求語言影響，一律英文
- [x] 驗證碼僅在 provider ≠ none、兩把 key 非空、對應開關（`on_register`、`on_login` 或 `on_create`）為 true 時啟用；非空環境變數覆蓋後台設定
- [x] 驗證碼逾時、非 2xx 或無法解析時一律拒絕，並套用於網頁註冊、網頁登入、網頁建立、`POST /api/v1/auth/register` 與 `POST /api/v1/auth/login`（400 `captcha_failed`）
- [x] 回報的 `hostname` 僅在符合 `URLET_BASE_URL` 的主機名稱或 `URLET_CAPTCHA_ALLOWED_HOSTNAMES` 時接受；未帶或空字串時接受，不符時拒絕並記一筆 `WARN`
- [x] siteverify 是系統唯一的對外 HTTP 請求，透過 `ureq` 2 + rustls + webpki-roots（不需 OpenSSL）發出，且預設關閉
- [x] `POST /api/v1/urls` 僅在 API Token 呼叫時豁免驗證碼（Session Cookie 呼叫受 `on_create` 約束）；後台 secret key 欄位為唯寫
- [x] `GET /` 對未登入與已登入使用者皆回 200 的導覽首頁，不再 302 至 `/app`
- [ ] `cargo test`（完整測試套件）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 全數通過，`scripts/smoke.sh` 端對端檢查全過，且容器映像可在 musl 下建置並執行（§3.3）

最後一項維持未勾選：上述功能已在原始碼中確認，但更新本文件時未執行測試套件與映像建置。

v1.2 新增項目：

- [x] `user_groups` 與 `group_domains`（16 張表、31 個索引）具備可空的 `max_urls` / `max_api_tokens`、七個功能開關、各群組預設網域與允許清單，並新增 `users.group_id` 與 `invite_codes.group_id`
- [x] 遷移建立 `Standard` 群組（兩個配額皆為 `NULL`）並把所有既有使用者指派進去，升級後行為不變
- [x] 群組值為 `NULL` 時生效配額跟隨全站設定，`0` 代表無上限；管理員與一般使用者適用同一套規則
- [x] 功能開關在建立時把關；更新時拒絕新開啟被封鎖的功能，但允許清除或維持原值
- [x] 管理員可建立、編輯、刪除群組並轉移唯一的預設標記；刪除非預設群組必須指定改派目標，預設群組不可刪除
- [x] 邀請碼可指定群組，兌換時優先採用；群組被刪除時透過 `ON DELETE SET NULL` 回退到預設群組
- [x] 群組網域選擇是一項開關：生效的群組預設網域一律允許；關閉時建立或移動到其他網域會被拒絕，且兩個表單都會隱藏網域下拉
- [x] 群組可設定自己的預設網域，並用 `group_domains` 限制可選網域；開啟限制但沒有資料列時只能用預設，預設網域停用/刪除時回退全站預設
- [x] Token 配額預設為 20 個可用 Token，顯示於帳號頁，並與生效連結配額及開關一起在 `GET /api/v1/auth/me` 回傳
- [x] 群組異動寫入稽核（`admin.group_created` / `updated` / `deleted`），使用者改派記錄於稽核 metadata
- [x] `cargo test`（374 個測試，1 個 ignored）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 全數通過，`scripts/smoke.sh` 108 項檢查全過（含群組 CRUD、配額強制、網域策略與改派）

v1.4 新增項目：

- [x] `themes`（19 張表、35 個索引）：六個內建設計與管理員 CSS 上傳、啟用/預設切換、全站切換權限、`urlet_theme` cookie、各主題模板目錄，以及持久化於 `/data/themes` 的 `custom_themes` 目錄與檔案

v1.5 新增項目：

- [x] `urls.redirect_mode`（`direct` / `splash`）、`splash_delay_seconds`（1–60，預設 5）、`splash_title`（≤120）與 `splash_message`（≤500，可換行）；既有連結維持 `direct`
- [x] 中轉頁連結回 200 中間頁，顯示自訂或在地化文案、目的地主機、倒數、meta refresh 與立即前往按鈕；送出時記錄點擊、設定 `no-store`，且不載入外部資源
- [x] 受密碼保護的連結以密碼頁為唯一中間頁，解鎖後直接前往目標
- [x] `user_groups.redirect_mode_policy`（`direct` / `splash` / `choice`，預設 `choice`）只把關建立與模式切換，透過 `GET /api/v1/auth/me` 曝光，兩個網頁表單都會隱藏被封鎖的選項
- [x] 六個主題皆提供中轉頁模板，三份語言目錄涵蓋所有新字串
- [x] `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`scripts/smoke.sh` 與 `scripts/merge-check.sh` 全綠

v1.6 新增項目：

- [x] 只要訪客可以建立連結，canonical 導覽首頁就提供公開短網址工具；額外（僅轉址）網域永遠不會，且該處的 `POST /` 回 `404`
- [x] `public_create_access`（`guest` / `user` / `admin`，預設 `user`）把關每一條建立路徑；當呼叫者低於該層級時，儀表板會以提示取代表單
- [x] 保留的訪客帳號（`users.is_system = 1`，19 張表／36 個索引）擁有匿名連結；它不會出現在使用者列表與任何計數中，在管理員的網址列表標為 `Guest`，無法登入，也不能編輯或刪除
- [x] `public_allow_*` 開關決定公開表單提供哪些選填欄位；送出時填了被停用的欄位會回 `403`
- [x] `public_captcha` 保護匿名流程，已登入的 Session 依 `captcha_on_create`，Bearer Token 則維持豁免
- [x] `public_rate_limit_per_ip` 與 `public_rate_limit_global` 只限制匿名建立，週期可各自選擇分鐘／時／日／週／月（預設每自然小時 20／500 次），`0` 會停用任一項，被拒絕時回 `429` / `?err=rate_limited`
- [x] `POST /api/v1/public/urls` 回 `201 {domain, slug, short_url, target_url, created_at}`，且不提供連結 id
- [x] `?shortened=<slug>` 以資料庫解析，因此結果框永不逐字印出查詢字串的值，也永不洩漏目的地
- [x] 三份語言目錄與六個導覽首頁模板都帶有新字串與共用的表單 partial
- [x] `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`scripts/smoke.sh` 與 `scripts/merge-check.sh` 全綠

---

## 12. 未來擴充路徑（本版僅保留 schema/架構空間）

| 功能 | 已預留 |
|------|--------|
| 各使用者的網域例外 | `user_groups` + `group_domains` 已提供各群組預設與允許清單；可新增 `user_domains` 表提供單一帳號的例外 |
| 細緻權限 | v1.2 已具備 `user_groups`、`users.group_id` 與七個功能開關；未來可新增 `group_permissions` 表加入逐動作權限 |
| 團隊協作 / 共享工作區 | `users.group_id` 已能將帳號分組；可新增 `url_members` 把 `urls.user_id` 的所有權擴充到群組或團隊 |
| 密碼重設 / Email 驗證 | `users.email_verified_at`、可新增 `password_resets` |
| GeoIP 國家統計 | `clicks.country_code` 已存在，只需加入解析器 |
| 點擊明細匯出 / CSV | `clicks` 完整欄位 + 索引 |
| 稽核合規 | `audit_logs.metadata` JSON 保留完整上下文 |
