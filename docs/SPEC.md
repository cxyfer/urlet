# uwurls — 輕量化多用戶短網址系統規格書 (Spec)

> 版本：1.0
> 狀態：Approved for implementation
> 最後更新：2026-02-11

---

## 1. 概述 (Overview)

`uwurls` 是一套**自託管、多用戶**的短網址服務。每位使用者擁有自己獨立的短網址空間，
可建立短網址、設定**密碼保護**與**過期機制**、檢視點擊統計，並透過個人 API Token 以程式方式操作。

系統以「**盡可能輕量**」為最高設計原則：單一 Rust 二進位檔 + 單一 SQLite 檔案，
一個 `docker compose up -d` 即可完成部署，不需要外部資料庫、快取或前端建置流程。

### 1.1 核心設計原則 (來自需求方)

| # | 原則 | 落實方式 |
|---|------|----------|
| P1 | 多用戶，各自獨立管理自己的網址 | 每個資源查詢都強制帶入 `user_id` 所有權過濾；跨用戶存取一律 404 |
| P2 | 密碼保護 | 短網址層級 Argon2id 密碼雜湊 + 簽章解鎖 Cookie（TTL 30 分鐘） |
| P3 | 過期機制 | 絕對時間 `expires_at` + 點擊次數上限 `max_clicks`，過期後回 410 Gone |
| P4 | Docker Compose 部署 | 單一 `docker-compose.yml`（app + named volume），可選 Caddy TLS overlay |
| P5 | 盡可能輕量化 | 單一二進位、SQLite（無 MySQL/Redis）、SSR 模板（無 Node 建置）、映像檔 < 40 MB |
| P6 | Rust 實作 | Rust 2021 + Axum 0.8 + Tokio + sqlx(SQLite) |
| P7 | 完善的資料表設計 | 13 張表、完整索引、軟刪除、稽核日誌、時間序列彙總表，便於未來擴充 |
| P8 | 適度添加功能 | 標籤、點擊分析、API Token、邀請碼、管理後台、稽核日誌、每日彙總 |

### 1.2 與先前半成品 (OURLS / OURLS2) 的關係

先前專案已探索過兩個方向，本專案**繼承其資料模型與功能構想**，但依 P5 重新選型：

| 項目 | OURLS（半成品） | OURLS2（半成品） | **uwurls（本專案）** |
|------|----------------|------------------|----------------------|
| 執行環境 | Axum + MySQL + Redis | Cloudflare Workers (WASM) + D1 | **Axum + SQLite（單二進位）** |
| 部署 | docker compose（3 容器） | Wrangler / Cloudflare | **docker compose（1 容器）** |
| 認證 | JWT + Refresh Token + OAuth | JWT + OAuth | **伺服器端 Session + API Token** |
| 前端 | React + Vite + Tailwind（需 Node 建置） | React + Vite + Pages | **Askama SSR + 原生 CSS/JS（零建置）** |
| 權限 | 使用者群組 + 權限矩陣 | 群組 + 權限矩陣 | **角色 (user/admin) + 狀態** |
| 統計 | 完整（GeoIP、MaxMind） | 完整 | **精簡（UA 解析、每日彙總、無 GeoIP）** |
| 繼承 | 密碼保護、過期、點擊上限、標籤、API Token、邀請碼、稽核日誌 | 同上 + 公開預覽 API | 上述全部，去蕪存菁 |

**刻意不做（Non-Goals）**：OAuth 第三方登入、使用者群組權限矩陣、GeoIP/MMDB、
關鍵字黑名單、A/B 測試、Webhook、QR Code、Link-in-bio、多語系、郵件寄送。

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

---

## 3. 系統架構

### 3.1 架構圖

```
                         ┌──────────────────────────────────────────┐
   Browser / curl        │            uwurls (單一容器)              │
  ───────────────────►   │                                          │
   :8080                 │  Axum Router                             │
                         │   ├─ /{slug}            跳轉 / 密碼頁     │
                         │   ├─ /api/v1/**         JSON API         │
                         │   ├─ /app/**            SSR 管理介面      │
                         │   ├─ /assets/**         內嵌 CSS/JS       │
                         │   └─ /health            健康檢查          │
                         │                                          │
                         │  Middleware: 安全標頭 → 請求 ID → 日誌     │
                         │              → Session/Token 認證         │
                         │              → CSRF → 速率限制            │
                         │                                          │
                         │  Services: slug / url / click / ua /      │
                         │            ratelimit / audit / settings   │
                         │                                          │
                         │  sqlx Pool (WAL)  ──►  /data/uwurls.db    │
                         └──────────────────────────────────────────┘
                                       named volume: uwurls-data
```

### 3.2 技術選型與理由

| 層級 | 選擇 | 理由（相對於先前半成品） |
|------|------|--------------------------|
| HTTP | `axum` 0.8 + `tokio` | 生態成熟、型別安全、單一二進位 |
| DB | `sqlx` 0.8 + SQLite（`bundled`） | 免外部服務、單檔備份、WAL 併發讀取足夠自託管規模 |
| 遷移 | `sqlx::migrate!` 內嵌 SQL | 編譯期嵌入，啟動自動遷移，無需 CLI |
| 模板 | `askama` 0.12 | 編譯期檢查的 SSR，無 Node/Vite 建置流程 |
| 靜態資產 | `include_str!` 內嵌 CSS/JS | 二進位自帶資源，映像檔不需檔案系統 |
| 密碼 | `argon2` (Argon2id) | 業界標準，抗 GPU 暴力破解 |
| Session/Token | 自製 `sha2` 雜湊 + 隨機 `getrandom` | 不引入 JWT 相依；伺服器端可即時撤銷 |
| 日誌 | `tracing` + `tracing-subscriber` | 結構化日誌、`RUST_LOG` 控制 |
| 時間 | `time` 0.3 | 較 `chrono` 輕，格式化 ISO-8601 足夠 |

**刻意避免的相依**：Redis、MySQL client、jsonwebtoken、reqwest（不對外抓取目標網址標題 → 同時消除 SSRF 風險）、
MaxMind/GeoIP、React/Node 工具鏈。

### 3.3 二進位檔與映像檔尺寸目標

| 項目 | 目標 | 實測 |
|------|------|------|
| Release 二進位（stripped, LTO, `opt-level=z`） | ≤ 15 MB | **4.2 MB**（glibc 本機）／4.5 MB（容器內 musl） |
| 執行期映像（`alpine` + 二進位 + CA 憑證） | ≤ 40 MB | **13.2 MB** |
| 閒置／負載記憶體 | ≤ 30 MB RSS | **6.8 MB**（500 次請求後） |
| 冷啟動到可服務 | ≤ 1 秒 | **約 0.2 秒**（含遷移） |

---

## 4. 功能需求

### 4.1 使用者與認證 (FR-AUTH)

| ID | 需求 | 說明 |
|----|------|------|
| FR-AUTH-1 | 註冊 | Email + 密碼。註冊模式由設定 `registration_mode` 控制：`open` / `invite` / `closed` |
| FR-AUTH-2 | 登入 | Email + 密碼，成功後建立 Session（HttpOnly Cookie，預設 TTL 7 天，滑動續期） |
| FR-AUTH-3 | 登出 | 撤銷當前 Session（`revoked_at`）並清除 Cookie |
| FR-AUTH-4 | 修改密碼 | 需驗證當前密碼；成功後撤銷該使用者其他所有 Session |
| FR-AUTH-5 | 個人資料 | 修改顯示名稱；檢視 Email、角色、建立時間 |
| FR-AUTH-6 | Session 管理 | 檢視自己的登入裝置清單，可撤銷任一 Session |
| FR-AUTH-7 | 首個使用者 | 若資料庫無任何使用者，第一位註冊者自動成為 `admin`；亦可用 `UWURLS_ADMIN_EMAIL/PASSWORD` 於啟動時建立 |
| FR-AUTH-8 | 停權 | `status = suspended` 的使用者無法登入，既有 Session 立即失效 |
| FR-AUTH-9 | 速率限制 | 登入/註冊/解鎖具記憶體令牌桶限制，並記錄 `login_attempts` 供稽核 |

**密碼政策**：長度 8–128 字元；Argon2id 參數 `m=19456 KiB, t=2, p=1`（OWASP 建議值）。

### 4.2 短網址管理 (FR-URL)

| ID | 需求 | 說明 |
|----|------|------|
| FR-URL-1 | 建立 | 必填 `target_url`；可選 `slug`、標題、備註、標籤、密碼、過期時間、點擊上限、轉址類型 |
| FR-URL-2 | 自動 Slug | 未指定時產生 7 字元 base62 隨機 Slug，碰撞時重試（最多 5 次） |
| FR-URL-3 | 自訂 Slug | 長度 1–64，僅 `[A-Za-z0-9_-]`，不得為保留字，全域唯一 |
| FR-URL-4 | 編輯 | 可改 target、標題、備註、標籤、密碼（設定/清除）、過期、點擊上限、轉址類型、狀態 |
| FR-URL-5 | 刪除 | 軟刪除（`deleted_at`），Slug 保留不釋出（避免連結被劫持），點擊紀錄保留 |
| FR-URL-6 | 列表 | 分頁、關鍵字搜尋（slug/target/標題）、標籤篩選、狀態篩選、排序（建立時間/點擊數） |
| FR-URL-7 | 狀態切換 | `active` / `disabled`；`disabled` 不轉址（回 404 頁面） |
| FR-URL-8 | 標籤 | 使用者層級標籤，名稱唯一（同使用者內），建立網址時自動建立缺少的標籤 |
| FR-URL-9 | 配額 | 每位使用者網址數量上限由設定 `max_urls_per_user` 控制（0 = 無限制） |
| FR-URL-10 | 所有權隔離 | 所有 `/api/v1/urls*` 查詢強制 `user_id = current_user`；他人資源一律 404 |

### 4.3 密碼保護 (FR-PWD)

| ID | 需求 | 說明 |
|----|------|------|
| FR-PWD-1 | 設定 | 建立/編輯時提供明文密碼，系統以 Argon2id 雜湊存入 `urls.password_hash` |
| FR-PWD-2 | 存取 | 受保護的短網址直接訪問時回傳 200 + 密碼輸入頁（**不洩漏目標網址**） |
| FR-PWD-3 | 解鎖 | `POST /{slug}/unlock` 驗證密碼；成功後設定簽章 Cookie `uwurls_unlock_<slug>`（HMAC-SHA256，TTL 30 分鐘） |
| FR-PWD-4 | 已解鎖 | 持有效解鎖 Cookie 時直接轉址，並記錄 `password_protected = 1` |
| FR-PWD-5 | 速率限制 | 每個 (slug, IP) 5 分鐘內最多 10 次嘗試，超過回 429 |
| FR-PWD-6 | 錯誤處理 | 密碼錯誤回 401 並在頁面上顯示錯誤，不揭露目標網址 |

### 4.4 過期機制 (FR-EXP)

| ID | 需求 | 說明 |
|----|------|------|
| FR-EXP-1 | 時間過期 | `expires_at`（ISO-8601 UTC）。到達後短網址失效，回 **410 Gone** 頁面 |
| FR-EXP-2 | 點擊上限 | `max_clicks` 非空且 `click_count >= max_clicks` 時失效，回 410 |
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
| FR-REDIR-5 | 隱私 | 僅存 `ip_hash = HMAC(secret, ip)` 與 `visitor_hash = HMAC(secret, ip + ua + 日期)`；**不存原始 IP** |
| FR-REDIR-6 | 唯一訪客 | 同一 `visitor_hash` 於同一 URL 首次點擊記 `is_unique = 1` |
| FR-REDIR-7 | 快取控制 | 回應帶 `Cache-Control: no-store`（受保護/過期/停用）；正常轉址 `private, max-age=0` |
| FR-REDIR-8 | 統計開關 | 設定 `analytics_enabled = false` 時不寫入點擊 |

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
| FR-TOK-1 | 建立 | 產生 `uw_` + 32 bytes base62；**僅在建立時顯示一次**明文 |
| FR-TOK-2 | 儲存 | 僅存 SHA-256 雜湊 + 前 8 字元前綴（供辨識） |
| FR-TOK-3 | 使用 | `Authorization: Bearer <token>`；與 Session 認證並存 |
| FR-TOK-4 | 生命週期 | 可設 `expires_at`、可撤銷（`revoked_at`）、記錄 `last_used_at` |
| FR-TOK-5 | 範圍 | `scopes` 欄位保留（v1 提供 `urls:read`、`urls:write`、`admin`），v1 僅檢查是否為 admin |

### 4.8 管理後台 (FR-ADMIN)

| ID | 需求 | 說明 |
|----|------|------|
| FR-ADMIN-1 | 使用者管理 | 列表、搜尋、改角色、停權/復權、重設密碼、軟刪除 |
| FR-ADMIN-2 | 全站網址 | 列表、搜尋、強制停用/刪除 |
| FR-ADMIN-3 | 邀請碼 | 建立（次數/到期/備註）、列表、撤銷；`invite` 模式下註冊需邀請碼 |
| FR-ADMIN-4 | 全站設定 | 編輯 `settings` 表（站名、註冊模式、配額、預設轉址類型、統計開關、保留天數） |
| FR-ADMIN-5 | 稽核日誌 | 記錄登入、註冊、網址異動、管理操作、設定變更；可依動作/對象篩選 |
| FR-ADMIN-6 | 系統資訊 | 版本、資料庫大小、使用者/網址/點擊總數、運行時間 |
| FR-ADMIN-7 | 授權 | 所有 `/api/v1/admin/**` 與 `/app/admin/**` 需 `role = admin`，否則 403 |

### 4.9 JSON API (FR-API)

- 前綴 `/api/v1`，回應 `application/json`。
- 統一錯誤格式：
  ```json
  { "error": { "code": "validation_error", "message": "slug is invalid", "details": { "field": "slug" } } }
  ```
- 錯誤代碼：`bad_request` `unauthorized` `forbidden` `not_found` `conflict` `gone` `validation_error` `rate_limited` `internal_error`
- 分頁格式：`{ "items": [...], "page": 1, "per_page": 20, "total": 42, "total_pages": 3 }`
- 認證：Session Cookie **或** `Authorization: Bearer <api_token>`。
- 端點清單見 §7。

### 4.10 網頁介面 (FR-UI)

| 路徑 | 頁面 | 說明 |
|------|------|------|
| `/app/login` | 登入 | Email + 密碼 |
| `/app/register` | 註冊 | 依註冊模式顯示邀請碼欄位或關閉提示 |
| `/app` | 儀表板 | 統計卡片、每日趨勢圖、網址列表（搜尋/篩選/分頁）、建立表單 |
| `/app/urls/{id}` | 網址詳情 | 編輯表單、QR 免（Non-Goal）、統計圖表、最近點擊、刪除 |
| `/app/account` | 帳號 | 個人資料、修改密碼、Session 清單、API Token 管理 |
| `/app/admin` | 管理後台 | 使用者、全站網址、邀請碼、設定、稽核日誌 |
| `/{slug}` | 轉址 / 密碼頁 / 410 / 404 | 公開 |

**UI 技術**：Askama 模板 + 手寫 CSS（含 `prefers-color-scheme` 深色模式）+ 約 150 行原生 JS
（複製到剪貼簿、確認對話框、表單增強）。**不使用**任何前端框架或 CDN 資源（CSP 可嚴格限制為 `'self'`）。

---

## 5. 非功能需求

### 5.1 安全性 (NFR-SEC)

| ID | 需求 |
|----|------|
| NFR-SEC-1 | 密碼：Argon2id；Session/Token：僅存雜湊；比較使用常數時間比較 |
| NFR-SEC-2 | Cookie：`HttpOnly`、`SameSite=Lax`、`Secure`（當 `UWURLS_SECURE_COOKIES=true` 或 Base URL 為 https） |
| NFR-SEC-3 | CSRF：狀態變更請求檢查 `Origin`/`Referer` 同源；Cookie 認證 + 非 GET 一律驗證；Bearer Token 請求豁免 |
| NFR-SEC-4 | 安全標頭：`Content-Security-Policy: default-src 'self'; frame-ancestors 'none'`、`X-Content-Type-Options: nosniff`、`Referrer-Policy: strict-origin-when-cross-origin`、`X-Frame-Options: DENY`、https 時 `Strict-Transport-Security` |
| NFR-SEC-5 | 開放轉址防護：目標僅允許 `http` / `https`；拒絕 `javascript:`、`data:`、`file:` 等 |
| NFR-SEC-6 | 保留字防護：`api`、`app`、`assets`、`health`、`static`、`favicon.ico` 等不得作為 Slug |
| NFR-SEC-7 | 錯誤訊息不洩漏內部細節；登入失敗統一回應「Email 或密碼錯誤」 |
| NFR-SEC-8 | 使用者列舉防護：註冊重複 Email 回 409（可接受，屬常見取捨）；登入統一錯誤訊息 |
| NFR-SEC-9 | 速率限制：登入 10 次/5 分鐘/IP、註冊 5 次/小時/IP、解鎖 10 次/5 分鐘/(slug,IP) |
| NFR-SEC-10 | SQL 一律參數化（sqlx bind），無字串拼接 |
| NFR-SEC-11 | 稽核日誌涵蓋所有敏感操作 |
| NFR-SEC-12 | 啟動時若 `UWURLS_SECRET` 未設定，於資料目錄產生並持久化隨機密鑰（權限 0600） |

### 5.2 效能 (NFR-PERF)

| ID | 需求 |
|----|------|
| NFR-PERF-1 | 轉址路徑：單次 `SELECT`（命中索引）+ 佇列化寫入；P95 < 10 ms（實測 0.74 ms） |
| NFR-PERF-2 | SQLite：`journal_mode=WAL`、`synchronous=NORMAL`、`foreign_keys=ON`、`busy_timeout=5000` |
| NFR-PERF-3 | 連線池：`max_connections=8`；點擊寫入由單一背景 writer 序列化（避免 WAL 因並發寫入無法 checkpoint 而無限成長） |
| NFR-PERF-4 | 列表查詢一律分頁（預設 20，上限 100） |
| NFR-PERF-5 | 統計查詢使用 `url_daily_stats` 與索引，避免全表掃描 |

### 5.3 運維 (NFR-OPS)

| ID | 需求 |
|----|------|
| NFR-OPS-1 | `docker compose up -d` 即完成部署；首次啟動自動遷移 + 建立管理員 |
| NFR-OPS-2 | 健康檢查：`GET /health`（200 JSON）與 `uwurls healthcheck` 子命令（供 Docker HEALTHCHECK 使用） |
| NFR-OPS-3 | 資料持久化於 named volume `uwurls-data`，路徑 `/data` |
| NFR-OPS-4 | 日誌輸出 stdout（`tracing`，JSON 或 compact 由 `UWURLS_LOG_FORMAT` 控制） |
| NFR-OPS-5 | 設定全部來自環境變數（12-factor），`.env.example` 完整說明 |
| NFR-OPS-6 | 每日維護任務：清理過期 Session、可選清理過期點擊紀錄 |
| NFR-OPS-7 | 備份 = 複製 `uwurls.db`（WAL checkpoint 後），README 說明 |
| NFR-OPS-8 | 非 root 使用者執行容器；唯讀根檔案系統可運作 |

### 5.4 可維護性 (NFR-MAINT)

| ID | 需求 |
|----|------|
| NFR-MAINT-1 | 分層：`routes/`（HTTP）→ `services/`（業務）→ `db/`（SQL）；不跨層直接查詢 |
| NFR-MAINT-2 | 單一錯誤型別 `AppError`，集中轉換為 HTTP 回應 |
| NFR-MAINT-3 | 所有 SQL 集中於 `src/db/` 各模組，使用執行期查詢（無需 `DATABASE_URL` 即可編譯） |
| NFR-MAINT-4 | `cargo clippy -- -D warnings` 與 `cargo fmt --check` 通過 |
| NFR-MAINT-5 | 單元測試 + 整合測試（含 `tower::ServiceExt::oneshot` 端到端）覆蓋核心流程 |

---

## 6. 資料模型

### 6.1 ER 概觀

```
users ─┬─< sessions
       ├─< api_tokens
       ├─< urls ─┬─< clicks
       │         ├─< url_daily_stats
       │         └─< url_tags >─ tags
       ├─< tags
       ├─< invite_redemptions >─ invite_codes
       └─< audit_logs (actor)
domains ─< urls
settings (key-value)
login_attempts (稽核用)
```

### 6.2 設計原則

1. **時間欄位**：全部 `TEXT` 存 ISO-8601 UTC（`YYYY-MM-DDTHH:MM:SS.sssZ`），可直接字串比較與排序。
2. **主鍵**：`INTEGER PRIMARY KEY AUTOINCREMENT`（SQLite rowid 最佳化）。
3. **軟刪除**：`users.deleted_at`、`urls.deleted_at`；查詢一律加 `deleted_at IS NULL`。
4. **枚舉**：以 `TEXT + CHECK` 約束，保留未來擴充彈性（SQLite 無原生 ENUM）。
5. **布林**：`INTEGER 0/1`。
6. **彙總表**：`url_daily_stats` 為統計查詢的加速層，寫入時 upsert。
7. **保留擴充欄位**：`api_tokens.scopes`、`urls.notes`、`domains` 表、`audit_logs.metadata`。

### 6.3 DDL（`migrations/0001_init.sql` 摘要）

> 完整且唯一真實來源為 `migrations/0001_init.sql`；以下為結構摘要。

| 表 | 主要欄位 | 用途 |
|----|----------|------|
| `users` | id, email, email_normalized(UNIQUE), password_hash, display_name, role, status, email_verified_at, last_login_at, created_at, updated_at, deleted_at | 使用者 |
| `sessions` | id, user_id, token_hash(UNIQUE), user_agent, ip_hash, created_at, last_seen_at, expires_at, revoked_at | 登入狀態 |
| `api_tokens` | id, user_id, name, token_prefix, token_hash(UNIQUE), scopes, last_used_at, expires_at, created_at, revoked_at | API 憑證 |
| `domains` | id, hostname(UNIQUE), is_default, is_verified, created_by, created_at | 多網域預留（v1 僅預設網域） |
| `urls` | id, user_id, domain_id, slug(UNIQUE), target_url, title, notes, password_hash, expires_at, max_clicks, click_count, unique_click_count, redirect_type, status, is_custom_slug, last_clicked_at, created_at, updated_at, deleted_at | 短網址 |
| `tags` | id, user_id, name, color, created_at, UNIQUE(user_id,name) | 標籤 |
| `url_tags` | url_id, tag_id (PK) | 標籤關聯 |
| `clicks` | id, url_id, clicked_at, ip_hash, visitor_hash, user_agent, referrer, referrer_host, country_code, device_type, browser, os, is_bot, is_unique, password_protected | 點擊明細 |
| `url_daily_stats` | url_id, day, clicks, unique_clicks, bot_clicks (PK: url_id+day) | 每日彙總 |
| `audit_logs` | id, actor_user_id, actor_role, action, target_type, target_id, metadata, ip_hash, created_at | 稽核 |
| `settings` | key(PK), value, updated_at, updated_by | 全站設定 |
| `invite_codes` | id, code_hash(UNIQUE), code_prefix, note, created_by, max_uses, used_count, expires_at, status, created_at | 邀請碼 |
| `invite_redemptions` | id, invite_id, user_id, redeemed_at, UNIQUE(invite_id,user_id) | 邀請碼使用紀錄 |
| `login_attempts` | id, identifier_hash, ip_hash, success, created_at | 登入稽核/鎖定 |

### 6.4 索引策略

| 索引 | 目的 |
|------|------|
| `urls(slug)` UNIQUE | 轉址查詢（最熱路徑） |
| `urls(user_id, created_at DESC)` | 我的網址列表 |
| `urls(expires_at) WHERE expires_at IS NOT NULL` | 過期清理/篩選 |
| `clicks(url_id, clicked_at DESC)` | 單一網址統計 |
| `clicks(clicked_at)` | 保留期清理 |
| `sessions(user_id, expires_at)`、`sessions(expires_at)` | Session 驗證與清理 |
| `api_tokens(token_hash)` UNIQUE | Token 認證 |
| `url_daily_stats(day)` | 全站趨勢 |
| `audit_logs(created_at DESC)`、`audit_logs(actor_user_id, created_at DESC)`、`audit_logs(target_type,target_id)` | 稽核查詢 |
| `login_attempts(identifier_hash, created_at DESC)` | 暴力破解偵測 |

### 6.5 預設資料 (seed)

- `settings`：`site_name=uwurls`、`registration_mode=open`、`default_slug_length=7`、
  `max_urls_per_user=1000`、`default_redirect_type=302`、`analytics_enabled=true`、`click_retention_days=0`。
- `domains`：啟動時依 `UWURLS_BASE_URL` 的 host 插入/更新預設網域（`is_default=1`）。

---

## 7. API 規格

### 7.1 認證

| 方法 | 路徑 | 說明 |
|------|------|------|
| POST | `/api/v1/auth/register` | `{email, password, invite_code?}` → 201 `{user}` |
| POST | `/api/v1/auth/login` | `{email, password}` → 200 `{user}` + Set-Cookie |
| POST | `/api/v1/auth/logout` | 撤銷當前 Session → 204 |
| GET | `/api/v1/auth/me` | 當前使用者 → 200 `{user}` |
| PATCH | `/api/v1/auth/me` | `{display_name}` → 200 `{user}` |
| POST | `/api/v1/auth/password` | `{current_password, new_password}` → 204 |
| GET | `/api/v1/auth/sessions` | 我的 Session 清單 |
| DELETE | `/api/v1/auth/sessions/{id}` | 撤銷指定 Session → 204 |

### 7.2 短網址

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/urls` | 查詢參數：`q, tag, status, page, per_page, sort(created_at\|clicks)` |
| POST | `/api/v1/urls` | `{target_url, slug?, title?, notes?, tags?, password?, expires_at?, max_clicks?, redirect_type?}` |
| GET | `/api/v1/urls/{id}` | 單筆（含 `short_url`、`is_expired`、`has_password`） |
| PATCH | `/api/v1/urls/{id}` | 部分更新；`password: null` 清除密碼；`expires_at: null` 清除過期 |
| DELETE | `/api/v1/urls/{id}` | 軟刪除 → 204 |
| GET | `/api/v1/urls/{id}/stats?days=30` | 每日趨勢 + 維度分佈 + 摘要 |
| GET | `/api/v1/urls/{id}/clicks?page=1` | 點擊明細（分頁） |

### 7.3 標籤 / Token

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/tags` | 我的標籤（含使用次數） |
| DELETE | `/api/v1/tags/{id}` | 刪除標籤（僅解除關聯） |
| GET | `/api/v1/tokens` | 我的 API Token 清單（不含明文） |
| POST | `/api/v1/tokens` | `{name, expires_at?}` → 201 `{token: "uw_...", api_token}` **僅此一次** |
| DELETE | `/api/v1/tokens/{id}` | 撤銷 → 204 |

### 7.4 管理（需 admin）

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/api/v1/admin/users` | 使用者列表（`q, role, status, page`） |
| PATCH | `/api/v1/admin/users/{id}` | `{role?, status?, display_name?}` |
| POST | `/api/v1/admin/users/{id}/password` | `{new_password}` 重設 |
| DELETE | `/api/v1/admin/users/{id}` | 軟刪除（其網址一併軟刪除） |
| GET | `/api/v1/admin/urls` | 全站網址列表 |
| PATCH | `/api/v1/admin/urls/{id}` | 強制停用/啟用/刪除 |
| GET | `/api/v1/admin/invites` | 邀請碼列表 |
| POST | `/api/v1/admin/invites` | `{max_uses?, expires_at?, note?}` → 201 `{code: "inv_...", invite}` |
| DELETE | `/api/v1/admin/invites/{id}` | 撤銷 → 204 |
| GET | `/api/v1/admin/settings` | 全站設定 |
| PUT | `/api/v1/admin/settings` | 更新設定（白名單鍵） |
| GET | `/api/v1/admin/audit` | 稽核日誌（`action, target_type, actor, page`） |
| GET | `/api/v1/admin/stats` | 系統資訊與總量 |

### 7.5 公開

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/health` | `{status, version, database, uptime_seconds}` |
| GET | `/{slug}` | 轉址（302 等）或密碼頁（HTML）或 410/404 頁 |
| POST | `/{slug}/unlock` | 表單或 JSON `{password}`；成功 303 → `/{slug}` |
| GET | `/api/v1/public/urls/{slug}` | 公開中介資料：`{slug, has_password, is_expired, title?}`（**不含 target**） |

---

## 8. 設定（環境變數）

| 變數 | 預設 | 說明 |
|------|------|------|
| `UWURLS_BIND` | `0.0.0.0:8080` | 監聽位址 |
| `UWURLS_BASE_URL` | `http://localhost:8080` | 對外網址（用於產生短網址、CSRF 同源檢查、Cookie Secure 判斷） |
| `UWURLS_DATABASE_URL` | `sqlite:///data/uwurls.db?mode=rwc` | SQLite 連線字串 |
| `UWURLS_SECRET` | 自動產生並存檔 | Session/解鎖 Cookie/IP 雜湊的 HMAC 密鑰（≥ 32 字元） |
| `UWURLS_SECRET_FILE` | `<資料目錄>/secret.key` | 自動產生密鑰的存放位置 |
| `UWURLS_ADMIN_EMAIL` | — | 啟動時建立/升級的管理員 Email |
| `UWURLS_ADMIN_PASSWORD` | — | 上述管理員密碼（僅首次建立時使用） |
| `UWURLS_REGISTRATION` | 讀取設定表 | 覆寫註冊模式 `open\|invite\|closed` |
| `UWURLS_SECURE_COOKIES` | 依 `BASE_URL` 判斷 | 強制 Cookie `Secure` |
| `UWURLS_TRUST_PROXY` | `false` | 信任 `X-Forwarded-For` / `X-Forwarded-Proto` |
| `UWURLS_SESSION_TTL_HOURS` | `168` | Session 有效期（小時） |
| `UWURLS_UNLOCK_TTL_MINUTES` | `30` | 短網址解鎖 Cookie 有效期（分鐘） |
| `UWURLS_LOG` | `info` | `tracing` 過濾器 |
| `UWURLS_LOG_FORMAT` | `compact` | `compact` \| `json` |
| `UWURLS_DEFAULT_SLUG_LENGTH` | 讀取設定表 | 覆寫自動 Slug 長度（4–16） |

---

## 9. 部署

### 9.1 目錄結構

```
uwurls/
├── Cargo.toml  Cargo.lock  rustfmt.toml
├── Dockerfile  .dockerignore
├── docker-compose.yml            # 單一容器 + named volume
├── docker-compose.tls.yml        # 可選：Caddy 自動 HTTPS
├── deploy/Caddyfile
├── .env.example
├── migrations/0001_init.sql  migrations/0002_clicks_indexes.sql
├── docs/{SPEC.md,PLAN.md,API.md,DEPLOYMENT.md,screenshots/}
├── scripts/{smoke.sh,demo.sh,screenshot.py}
├── templates/                    # Askama 模板（base/app_base + 各頁面 + admin/）
├── src/
│   ├── main.rs  lib.rs  config.rs  state.rs  error.rs  server.rs  assets.rs
│   ├── assets/{app.css,app.js,favicon.svg}
│   ├── db/{mod,migrate,users,sessions,tokens,urls,tags,clicks,settings,invites,audit,domains,login_attempts}.rs
│   ├── services/{auth,urls,admin,slug,target,click,ua,unlock,cookies,chart,audit,settings,time,secret,password,ratelimit,tokens}.rs
│   ├── middleware/{mod,auth,csrf,security}.rs
│   └── routes/{mod,api,api_auth,api_urls,api_tags,api_tokens,api_stats,api_admin,api_public,redirect,health,assets,dto,extract}.rs
│       └── pages/{mod,auth,dashboard,urls,account,admin}.rs
└── tests/{common/mod.rs,auth.rs,urls.rs,redirect.rs,pages.rs,admin.rs}
```

### 9.2 Dockerfile 策略

多階段建置：
1. `rust:1-alpine` + `build-base` → `cargo build --release --target x86_64-unknown-linux-musl`（LTO、`strip`、`opt-level=z`）
2. `alpine:3.21` → 複製二進位 + `ca-certificates` + `tzdata`，建立非 root 使用者，`HEALTHCHECK CMD ["/usr/local/bin/uwurls","healthcheck"]`

### 9.3 Compose 服務

```yaml
services:
  uwurls:
    build: .
    image: uwurls:latest
    ports: ["8080:8080"]
    env_file: [.env]
    volumes: [uwurls-data:/data]
    healthcheck: ...   # uwurls healthcheck
    restart: unless-stopped
volumes:
  uwurls-data:
```

`docker-compose.tls.yml`：額外加入 Caddy（80/443、自動 Let's Encrypt），
uwurls 不對外暴露連接埠，由 Caddy 反向代理。

---

## 10. 測試策略

| 層級 | 範圍 | 工具 |
|------|------|------|
| 單元測試 | slug 產生/驗證/保留字、UA 解析、過期判定、設定解析、密碼雜湊、IP 雜湊、CSRF 同源判斷 | `#[cfg(test)]` 模組 |
| 整合測試 | 註冊→登入→建立→轉址→統計；密碼保護流程；過期與點擊上限；所有權隔離；admin 授權；API Token；CSRF 拒絕 | `tests/integration.rs` + `tower::ServiceExt::oneshot` + 臨時 SQLite |
| 端對端煙霧測試 | 以真實二進位 + curl 走完主要流程 | `scripts/smoke.sh` |
| 靜態檢查 | `cargo fmt --check`、`cargo clippy -D warnings` | CI 指令（README 說明） |

---

## 11. 驗收標準 (Definition of Done)

- [x] `cargo build --release` 成功，二進位 **4.2 MB** ≤ 15 MB
- [x] `cargo test` 全綠：**178** 項（單元 93 + 整合 84 + 二進位 1）
- [x] `cargo clippy --all-targets -- -D warnings` 與 `cargo fmt --check` 無警告
- [x] `scripts/smoke.sh` 對真實二進位 **81** 項全數通過（含密碼保護、過期、所有權隔離、CSRF、邀請碼、重啟持久化）
- [x] `docker compose config` 通過（base 與 TLS overlay 皆以 Docker Compose 27.4 驗證）
- [x] `docker build` 成功，映像 **13.2 MB**；`docker compose up -d` 後容器狀態 `healthy`
- [x] 容器內以 uid 10001 執行、根檔案系統唯讀、`cap_drop: ALL`；資料僅寫入 `/data` volume
- [x] 容器實測：health、管理員登入、建立含密碼/過期/點擊上限的短網址、解鎖轉址、全部管理頁面 200、重啟後 Session 與解鎖 Cookie 仍有效
- [x] 13 張表 + 索引依 §6 建立，啟動自動遷移（`schema_migrations` 有 checksum 保護）
- [x] 所有 FR/NFR 條目皆有對應實作或測試
- [x] README 可讓新使用者在 5 分鐘內完成部署
- [x] 每個階段皆有對應的本地 commit（見 `docs/PLAN.md`）

---

## 12. 未來擴充路徑（本版僅保留 schema/架構空間）

| 功能 | 已預留 |
|------|--------|
| 多網域 / 自訂網域 | `domains` 表 + `urls.domain_id` |
| 使用者群組與細緻權限 | `api_tokens.scopes`、`users.role` 可擴充為 `groups` 表 |
| 密碼重設 / Email 驗證 | `users.email_verified_at`、可新增 `password_resets` |
| GeoIP 國家統計 | `clicks.country_code` 已存在，只需加入解析器 |
| 點擊明細匯出 / CSV | `clicks` 完整欄位 + 索引 |
| 團隊協作 | 可新增 `url_members`，`urls.user_id` 為 owner |
| 稽核合規 | `audit_logs.metadata` JSON 保留完整上下文 |
