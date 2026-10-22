[English](../PLAN.md) | **繁體中文**

# urlet — 分階段實作計畫 (Implementation Plan)

> 對應規格：`docs/SPEC.md` v1.6
> 原則：每個階段結束時程式碼必須**可編譯、可測試、可提交**；每個階段一個（或數個）本地 commit。

## 執行結果（已完成）

| 階段 | 狀態 | Commit |
|------|------|--------|
| P0 Spec 與計畫 | ✅ 完成 | `4109fc8 📝 docs: add spec and phased implementation plan` |
| P1 專案骨架與資料層 | ✅ 完成 | `06f91a7 🎉 init: bootstrap urlet project skeleton` |
| P2 認證與 Session | ✅ 完成 | `e0d8902 ✨ feat(auth): add sessions, registration, login and CSRF protection` |
| P3 短網址核心 | ✅ 完成 | `e1db4fc ✨ feat(urls): add short link CRUD, slugs, tags and expiry rules` |
| P4 轉址與解鎖 | ✅ 完成 | `2a8ca77 ✨ feat(redirect): add redirects, password unlock and click analytics` |
| P5 Web UI | ✅ 完成 | `eae0e0b ✨ feat(ui): add server rendered dashboard, detail and account pages` |
| P6 管理後台與 API | ✅ 完成 | `c74302a ✨ feat(admin): add admin panel, invites, settings and maintenance` |
| P7 測試與文件 | ✅ 完成 | 見 `git log`（smoke.sh、README、API.md、DEPLOYMENT.md） |
| P8 最終驗證 | ✅ 完成 | 見 `git log`（release 建置、映像檔策略、收尾） |
| P10 使用者群組 (v1.2) | ✅ 完成 | 見 `git log`（migration 0004、群組 API/UI、三語字串、文件） |
| P11 群組網域策略 (v1.2) | ✅ 完成 | 見 `git log`（migration 0005、預設網域 + 允許清單、管理 UI/API、測試） |
| P12 可切換主題 (v1.4) | ✅ 完成 | 見 `git log`（migration 0010、主題 registry、管理目錄、六套模板、測試） |
| P13 中轉頁與轉址方式政策 (v1.5) | ✅ 完成 | 見 `git log`（migration 0011、六個主題的中轉頁模板、群組政策、三語字串、文件、測試） |
| P14 可上傳主題 (v1.5) | ✅ 完成 | 見 `git log`（migration 0012、CSS 上傳/編輯/刪除、雜湊資產、持久化 theme volume、測試） |
| P15 公開短網址工具 (v1.6) | ✅ 完成 | 見 `git log`（migration 0015、保留的訪客帳號、存取層級、欄位開關、兩組訪客額度、導覽首頁表單、公開 API、三語字串、文件、測試） |

驗收數據（v1.0）：`cargo test` 187 個測試全綠、`cargo clippy --all-targets -- -D warnings` 與
`cargo fmt --check` 無警告、`scripts/smoke.sh` 81 項端對端檢查全數通過。

---

## 里程碑總覽

| 階段 | 名稱 | 產出 | 驗收 | Commit |
|------|------|------|------|--------|
| P0 | Spec 與計畫 | `docs/SPEC.md`、`docs/PLAN.md` | 規格涵蓋所有需求、可追溯 | `📝 docs: add spec and phased implementation plan` |
| P1 | 專案骨架與資料層 | Cargo 專案、config、14 張表 migrations、db 模組、錯誤處理、health、Dockerfile、compose | `cargo run` 可啟動、`/health` 回 200、遷移成功 | `🎉 init: bootstrap urlet project skeleton` |
| P2 | 認證與 Session | 註冊/登入/登出、Argon2、Session、CSRF、速率限制、admin bootstrap | 整合測試涵蓋註冊登入、未授權 401、CSRF 403 | `✨ feat(auth): ...` |
| P3 | 短網址核心 | slug 服務、URL CRUD、標籤、過期/點擊上限/密碼欄位、配額 | 整合測試涵蓋 CRUD、所有權隔離、slug 驗證 | `✨ feat(urls): ...` |
| P4 | 轉址與解鎖 | redirect handler、點擊記錄、UA 解析、解鎖 Cookie、410/404 頁 | 整合測試涵蓋轉址、密碼流程、過期 410 | `✨ feat(redirect): ...` |
| P5 | Web UI | Askama 模板 + CSS/JS、儀表板、網址詳情、帳號頁 | 手動 + smoke 驗證頁面可操作 | `✨ feat(ui): ...` |
| P6 | 管理後台與 API | 使用者管理、邀請碼、設定、稽核、API Token、`/api/v1` 完整化 | 整合測試涵蓋 admin 授權與 Token | `✨ feat(admin): ...` |
| P7 | 測試與文件 | 整合測試補齊、`scripts/smoke.sh`、README、API.md、DEPLOYMENT.md | `cargo test` 全綠、smoke 全通過 | `🧪 test: ...` / `📝 docs: ...` |
| P8 | 最終驗證與收尾 | clippy/fmt 清零、release 建置、compose 設定驗證、映像檔策略確認 | 驗收清單全數勾選 | `🔖 chore: finalize v1.0.0` |
| P9 | 多語系、防機器人驗證與導覽首頁 | 三份內嵌語言目錄、語言協商與切換、註冊／登入／建立流程的 Turnstile／reCAPTCHA 驗證、`/` 導覽首頁 | 語言協商、cookie 切換、驗證碼失敗關閉（含登入）與導覽首頁皆有測試涵蓋；`/` 回 200 | `📝 docs: ...` / `✨ feat(i18n): ...` |
| P10 | 使用者群組 (v1.2) | `user_groups` 提供各群組連結／Token 配額與七個功能開關（含網域選擇）；可透過使用者或邀請碼指派；含管理頁面與 API | 16 張表／31 個索引、升級不改變行為、配額與開關生效（管理員亦然）、刪組需改派、測試與 smoke 全綠 | `✨ feat(groups): ...` |
| P11 | 群組網域策略 (v1.2) | 各群組 `default_domain_id`、`restrict_domains` 與 `group_domains` 允許清單；建立/移動連結時強制，透過 `/api/v1/auth/me` 與 `/api/v1/domains` 曝光，管理群組表單可編輯 | 16 張表／31 個索引、既有群組維持所有 active 網域、限制清單為空時只能用預設、預設網域停用時回退全站預設、測試與 smoke 全綠 | `✨ feat(domains): ...` |
| P12 | 可切換主題 (v1.4) | 六個內建模板／CSS 主題、`themes` 表與 registry、管理啟用／預設／政策頁面與 API、`urlet_theme` cookie、主題切換器與共享回退內容 | 18 張表／34 個索引、每個主題都能渲染每個頁面、權限與回退皆有測試、fmt/clippy/test/smoke 全綠 | `✨ feat(themes): ...` |
| P13 | 中轉頁與轉址方式政策 (v1.5) | `urls.redirect_mode` 與中轉頁文案、六個主題模板、`user_groups.redirect_mode_policy`、表單/API/`/auth/me`、語言目錄 | 既有連結維持直接跳轉、密碼頁仍是唯一中間頁、政策只把關建立與更新、fmt/clippy/test/smoke 全綠 | `✨ feat(splash): ...` |
| P14 | 可上傳主題 (v1.5) | `custom_themes` 中介資料、管理主題頁的 CSS 上傳/編輯/刪除、內容雜湊 `/assets/custom-themes/{key}`、`auto`/`light`/`dark` 外觀、驗證與限制、`URLET_THEMES_DIR` + `urlet-themes` volume | 19 張表／35 個索引、重載/重啟後上傳仍在、明暗強制、預設/刪除規則與成員授權皆有測試、fmt/clippy/test/smoke 全綠 | `✨ feat(themes): ...` |
| P15 | 公開短網址工具 (v1.6) | 導覽首頁的縮網址表單與 `POST /api/v1/public/urls`、擁有匿名連結的保留訪客帳號、`public_create_access`、八個 `public_allow_*` 欄位開關、`public_captcha`、每來源位址與全體訪客共用額度、管理設定區塊、三語字串、六個導覽首頁模板 | 19 張表／36 個索引、升級後訪客仍無法建立、存取層級把關每一條建立路徑、被停用的欄位由伺服器端拒絕、額度與訪客帳號皆有測試、fmt/clippy/test/smoke/merge-check 全綠 | `✨ feat(public): ...` |

---

## P1 — 專案骨架與資料層

**任務**

1. `Cargo.toml`：axum 0.8、tokio、sqlx(sqlite, runtime-tokio, migrate)、askama、argon2、sha2、hmac、time、serde、tracing、tower-http、rand/getrandom、base64、subtle、thiserror、anyhow、uuid？（不需要）
2. `[profile.release]`：`lto=true`、`codegen-units=1`、`strip=true`、`opt-level="z"`、`panic="abort"`
3. `src/config.rs`：環境變數解析 + 預設值 + 驗證（BASE_URL、DB 路徑、secret 自動產生）
4. `migrations/0001_init.sql`：§6.3 全部 14 張表 + 索引 + seed settings
5. `src/db/`：pool 建立（WAL/pragmas）、`migrate()`、各表 CRUD 模組骨架
6. `src/error.rs`：`AppError` 列舉 + `IntoResponse`（HTML 與 JSON 雙模式）
7. `src/server.rs` + `src/main.rs`：router、`/health`、`/assets/*`、優雅關閉
8. `Dockerfile`、`docker-compose.yml`、`docker-compose.tls.yml`、`deploy/Caddyfile`、`.env.example`、`.gitignore`
9. `rustfmt.toml`、`clippy` 設定

**驗收**：`cargo build` 成功；啟動後 `curl /health` 回 200 JSON；DB 檔自動建立且 14 張表存在。

---

## P2 — 認證與 Session

**任務**

1. `services/password.rs`：Argon2id 雜湊/驗證 + 常數時間比較
2. `services/secret.rs`：HMAC-SHA256 工具（IP 雜湊、解鎖 Cookie 簽章）
3. `services/ratelimit.rs`：記憶體令牌桶（key → bucket，定期清理）
4. `db/users.rs`、`db/sessions.rs`、`db/login_attempts.rs`
5. `services/auth.rs`：註冊（模式檢查、邀請碼）、登入、登出、修改密碼、Session 建立/驗證/撤銷
6. `middleware/auth.rs`：`CurrentUser` extractor（Session Cookie 或 Bearer Token）
7. `middleware/csrf.rs`：非 GET 且使用 Cookie 認證時檢查 `Origin`/`Referer`
8. `middleware/security.rs`：安全標頭；`middleware/request_id.rs`
9. `routes/api_auth.rs`：§7.1 端點
10. 首個使用者自動成為 admin；`URLET_ADMIN_EMAIL/PASSWORD` bootstrap

**驗收**：整合測試 — 註冊/登入/登出/`/me`/改密碼、未登入 401、跨來源 POST 403、連續失敗登入觸發 429。

---

## P3 — 短網址核心

**任務**

1. `services/slug.rs`：base62 產生、驗證（長度/字元/保留字）、碰撞重試
2. `services/url_target.rs`：目標網址正規化與 scheme 白名單（開放轉址防護）
3. `db/urls.rs`、`db/tags.rs`、`db/domains.rs`：CRUD、列表查詢（搜尋/標籤/狀態/排序/分頁）、配額檢查
4. `services/urls.rs`：建立/更新/刪除業務邏輯（含標籤 upsert、過期與上限驗證、密碼雜湊）
5. `routes/api_urls.rs`：§7.2 端點（`/stats`、`/clicks` 先回結構化資料）
6. `routes/api_tags.rs`：§7.3 標籤端點
7. 稽核日誌寫入（`services/audit.rs`）

**驗收**：整合測試 — CRUD、自訂/自動 slug、保留字拒絕、無效 target 拒絕、他人網址 404、配額上限、標籤篩選。

---

## P4 — 轉址與解鎖

**任務**

1. `routes/redirect.rs`：`GET /{slug}` 狀態機（不存在 404 / 刪除 404 / 停用 404 / 過期 410 / 需密碼 200 密碼頁 / 已解鎖 302）
2. `services/unlock.rs`：解鎖 Cookie 簽發與驗證（HMAC + slug 綁定 + TTL）
3. `routes/redirect.rs`：`POST /{slug}/unlock`（表單 + JSON）、速率限制
4. `services/ua.rs`：輕量 UA 解析（device/browser/os/bot）
5. `services/click.rs`：非同步寫入 `clicks` + `url_daily_stats` upsert + `click_count` 遞增 + 唯一訪客判定
6. 過期/404/密碼頁模板（Askama，最小版）
7. `middleware/security.rs`：轉址回應快取標頭

**驗收**：整合測試 — 正常轉址 302、自訂 301/307、密碼頁不洩漏 target、錯誤密碼 401、正確密碼後轉址、過期 410、達點擊上限 410、刪除/停用 404、點擊數與彙總正確增加。

---

## P5 — Web UI

**任務**

1. `templates/base.html` + `assets/app.css`（響應式、深色模式）+ `assets/app.js`（複製、確認、表單增強）
2. `templates/login.html`、`register.html`
3. `templates/dashboard.html`：統計卡片、SVG 趨勢圖、網址列表（搜尋/標籤/狀態/分頁）、建立表單（含進階選項：密碼、過期、點擊上限、標籤）
4. `templates/url_detail.html`：編輯、統計圖表、最近點擊、刪除
5. `templates/account.html`：個人資料、修改密碼、Session 清單、API Token 管理
6. `routes/pages_*.rs`：`/app/**` 路由 + 表單 POST handler（HTML 導向 + flash 訊息）
7. SVG 圖表產生器（`services/chart.rs`）

**驗收**：以真實二進位手動/腳本走完：註冊 → 建立（含密碼與過期）→ 複製短網址 → 詳情頁看到統計 → 帳號頁建立 Token。

---

## P6 — 管理後台與 API 完整化

**任務**

1. `middleware/admin.rs`：admin 授權
2. `routes/api_admin.rs`：§7.4 全部端點（使用者、全站網址、邀請碼、設定、稽核、系統資訊）
3. `routes/api_tokens.rs`：§7.3 Token 端點
4. `routes/api_public.rs`：公開中介資料端點
5. `templates/admin/*.html` + `/app/admin/**` 頁面
6. `db/settings.rs`：設定讀寫 + 快取（`RwLock` 記憶體快取，寫入失效）
7. 每日維護任務（清理過期 Session、點擊保留期清理）

**驗收**：整合測試 — 非 admin 存取 403、admin 可停權使用者（其 Session 立即失效）、邀請碼註冊流程、設定更新生效、稽核日誌有紀錄、API Token 認證成功。

---

## P7 — 測試與文件

**任務**

1. `tests/common/mod.rs`：測試 App 工廠（臨時 SQLite、記憶體設定）
2. `tests/integration.rs`：§10 全部整合測試案例
3. 單元測試補齊（slug、UA、過期、設定、CSRF、密碼、chart）
4. `scripts/smoke.sh`：真實二進位 + curl 端對端（含 410、解鎖、所有權隔離）
5. `README.md`（快速開始、部署、備份、升級）、`docs/API.md`、`docs/DEPLOYMENT.md`
6. `LICENSE`（MIT）

**驗收**：`cargo test` 全綠、`scripts/smoke.sh` 全通過。

---

## P8 — 最終驗證與收尾

**任務**

1. `cargo fmt`、`cargo clippy --all-targets -- -D warnings` 清零
2. `cargo build --release` 並量測二進位大小（目標 ≤ 15 MB）
3. `docker compose config` / YAML 驗證；確認 Dockerfile 邏輯正確（本機無 Docker 引擎時，以 YAML 解析 + 明確標註）
4. 以 release 二進位跑一次 smoke test 與健康檢查
5. 更新 `docs/PLAN.md` 勾選狀態、README 驗收清單

**驗收**：`docs/SPEC.md` §11 驗收清單全數勾選。

---

## P9 — 多語系、防機器人驗證與導覽首頁 (v1.1)

**任務**

1. 語言目錄 `src/i18n/{en,zh-Hant,zh-Hans}.json`（由 `src/i18n/mod.rs` 載入）：以 `include_str!` 載入目錄，依 `Accept-Language` 協商（正確處理 `q=` 權重、預設**英文**），缺少 key 時回退英文字串。
2. `src/middleware/locale.rs`：由 `urlet_lang` cookie 或 `Accept-Language` 決定當前語言（cookie 優先），透過 task-local context 提供給 handler 與錯誤頁面，並據以輸出 `<html lang>`，同時為 HTML 回應加上 `Content-Language` 與 `Vary: Accept-Language, Cookie`。
3. `GET /app/lang/{code}?next=<path>`：設定 cookie（`Path=/`、`SameSite=Lax`、`Max-Age` 一年、非 HttpOnly、`URLET_SECURE_COOKIES=true` 時加 `Secure`）後回 302；`next` 只接受同站相對路徑，其餘（含未帶 `next`）回退 `/app`；無法辨識的 `code` 視為 `en`。
4. `src/services/captcha.rs`：Cloudflare Turnstile 與 Google reCAPTCHA v2 的供應商抽象層，支援非空環境變數覆蓋後台設定，以表單編碼搭配 5 秒逾時、透過 `ureq` 2（rustls + webpki-roots，不需 OpenSSL）對供應商 siteverify 端點發出 HTTPS POST，並檢查回報的 `hostname` 是否為 base URL 的主機名稱或白名單之一（未帶 `hostname` 時通過，不符時拒絕並記 `WARN`），失敗一律拒絕（逾時／非 2xx／無法解析）。測試位於 `tests/captcha.rs`。
5. 於網頁註冊、登入與建立短網址表單，以及 `POST /api/v1/auth/register`、`POST /api/v1/auth/login`（JSON 欄位 `captcha_token`，失敗回 400 `captcha_failed`）套用驗證碼；`POST /api/v1/urls` 僅對 Session Cookie 呼叫套用，帶 `Authorization: Bearer ur_...` 者豁免，以維持自動化流程可用。
6. 管理後台設定：新增 `captcha_provider`、`captcha_site_key`、`captcha_secret_key`、`captcha_on_register`、`captcha_on_login`、`captcha_on_create`、`captcha_allowed_hostnames` 七個 key，各有一個對應的 `URLET_CAPTCHA_*` 環境變數（含 `URLET_CAPTCHA_ON_LOGIN` 與 `URLET_CAPTCHA_ALLOWED_HOSTNAMES`），非空時覆蓋表中值；secret key 欄位為唯寫（留白送出維持原值）。
7. `GET /` 導覽首頁（`src/routes/pages/home.rs` + `templates/landing.html`）：hero、功能亮點、登入與註冊入口、語言切換、頁尾，已登入者顯示進入儀表板的入口；移除 302 至 `/app`。
8. 測試：語言協商（`q=` 權重、回退英文）、cookie 切換與 `next` 驗證、API JSON 維持英文、驗證碼啟用條件矩陣、註冊／登入／建立時的驗證碼失敗關閉、`GET /` 回 200。
9. 文件：更新 `docs/SPEC.md` 與中文鏡像 `docs/zh-TW/SPEC.md`（FR 表、路由清單、環境變數、設定 key）、兩份 `PLAN.md`、兩份 README、`docs/API.md`、`docs/DEPLOYMENT.md`、其 `docs/zh-TW/` 鏡像與 `.env.example`。

**驗收**：`GET /` 對未登入與已登入使用者皆回 200 且不再轉址；`Accept-Language` 協商與語言 cookie 可切換介面，而 JSON API 回應維持英文；驗證碼僅在 provider ≠ `none`、兩把 key 非空、對應開關（`on_register`、`on_login` 或 `on_create`）為 true 時啟用，且逾時／非 2xx／解析失敗時在註冊、登入與建立流程一律拒絕；`POST /api/v1/urls` 對 Session Cookie 呼叫套用驗證碼，帶 `Authorization: Bearer ur_...` 者維持豁免；後台 secret key 永不回顯；`cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 全數通過。

---

## P10 — 使用者群組 (v1.2)

**任務**

1. `migrations/0004_user_groups.sql`：`user_groups`（可空的連結／Token 配額、七個開關含 `allow_domain_selection`、`is_default`）、`users.group_id`、`invite_codes.group_id`、預設唯一的部分索引，以及 `Standard` 預設群組並把既有使用者全部遷入；種下 `max_api_tokens_per_user=20`。
2. `src/db/user_groups.rs` + `src/services/limits.rs`：群組 CRUD、成員計數、改派刪除，以及生效額度解析（`NULL` 跟隨全站、`0` 無上限、開關只能收窄全站預設）。
3. `src/services/groups.rs` + `AdminContext`：驗證、重名／預設保護、具稽核的建立／更新／刪除、預設標記轉移，以及使用者與邀請碼的指派。
4. 強制點：建立／更新時的連結配額與七個開關、Token 開關與可用 Token 配額、註冊時的邀請碼群組（`open` 模式下亦會驗證）。
5. API：`/api/v1/admin/groups` CRUD、使用者與邀請碼的 `group_id`、`GET /api/v1/auth/me` 的 `group`／`limits`。
6. UI：`/app/admin/groups`、使用者頁的群組篩選與指派、邀請碼群組選單、Token 配額設定、儀表板／明細／帳號頁依群組調整，以及三語字串。
7. 測試：配額繼承與群組驗證的單元測試，以及 `tests/groups.rs` 涵蓋授權、CRUD、配額、開關、邀請碼、刪組、預設轉移與頁面渲染。
8. 文件：中英 SPEC／PLAN／API／README，以及群組的 smoke 檢查。

**驗收**：16 張表、31 個索引；種下的預設群組不改變任何既有行為；配額與開關依群組生效（管理員亦然）；刪除群組必須指定改派目標；`/me` 回傳生效額度；`cargo test`、clippy、fmt 與 smoke 全數通過。

---

## P11 — 群組網域策略（v1.2 follow-up）

**任務**

1. `migrations/0005_group_domains.sql`：`user_groups.default_domain_id`（可空、`ON DELETE SET NULL`）、`user_groups.restrict_domains`、`group_domains(group_id, domain_id)` 及其網域索引。
2. `services/domain_policy.rs`：解析生效預設（群組預設 active 時採用，否則全站預設）、允許清單與 `allow_domain_selection`，並提供過濾後的 `DomainOption`。
3. 連結服務：未指定 `domain` 時使用群組預設；建立/移動須通過 `allows()`；維持目前網域一律成功。
4. API/UI：group payload/DTO 欄位、`/api/v1/auth/me` 的 `default_domain` + `allowed_domains`、`/api/v1/domains` 過濾、管理群組表單的下拉與勾選。
5. 測試：v4 升級、空允許清單、明確允許清單、預設停用回退、關閉選擇、partial update、HTML 表單、API/UI 與 smoke。

**驗收**：既有群組維持所有 active 網域；受限群組只能建立/移動到允許清單加預設；限制清單為空時只能用預設；群組預設停用/刪除時回退全站預設；`cargo test`、clippy/fmt 與 smoke 全綠。

## P12 — 可切換主題 (v1.4)

**任務**

1. `migrations/0010_themes.sql`：`themes` 表、唯一預設 partial index、六筆種子資料與 `theme_switching=all` 設定。
2. `src/db/themes.rs` + `src/services/theme.rs`：主題 key、registry 快取、預設/回退解析、管理清單、切換權限與交易式選擇更新。
3. `src/routes/pages/themed.rs`：編譯期 `themed_page!` 包裝，讓每頁在六個 Askama 模板中選擇，且不信任請求提供的路徑；頁面 context 以 `.themed()` 加入。
4. Middleware 與資產：`urlet_theme` cookie 解析、`GET /app/theme/{key}`、`/assets/themes/{key}`、`/assets/theme-ui.css` 與共用切換器 macro。
5. 管理：`/app/admin/themes` 頁面、`GET/PUT /api/v1/admin/themes`、稽核 action、驗證（至少一個啟用、預設必須啟用）與快取重載。
6. 主題：將五個已關閉的設計提案（Aurora、Paper、Signal、Gilded、Swiss）匯入為模板目錄與 CSS；以 macro 共享 v1.2 頁面內容，讓配額、網域、群組、邀請碼與設定在每個主題都能運作。
7. 測試：registry 預設、管理 CRUD/政策、切換權限、停用/未知回退、資產路由，以及每個主題 × 每個頁面的完整渲染迴圈。
8. 文件：SPEC、PLAN、API、README 與 `zh-TW` 鏡像；`.env.example` 不需新增變數，因為主題完全由資料庫控制。

**驗收**：六個主題可選、沒有 cookie 時使用預設、停用主題回退、管理員可預覽任何已編譯主題（包含已停用者）、`all`/`admin` 切換權限生效、每個頁面在每個主題下都能渲染，且 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 與 `scripts/smoke.sh` 全綠。

---

## P14 — 可上傳主題 (v1.5)

上傳在不改變編譯期模板模型的前提下擴充主題目錄。

1. `migrations/0012_custom_themes.sql` + `0013_custom_theme_appearance.sql`：上傳樣式表的中介
   資料表，以產生的 `custom-…` key 識別；對應的 `themes` 資料列負責啟用、預設與排序，外觀
   migration 會轉換舊的僅深色旗標。
2. `src/db/custom_themes.rs`：以單一交易建立/更新/刪除中介資料與目錄資料列。
3. `src/services/custom_theme.rs`：CSS 驗證（UTF-8、大小、禁止外部資源）、`auto`／`light`／
   `dark` 外觀、原子寫檔、內容雜湊，以及與 `update_selection` 共用的 settings 鎖序列化。
4. `src/services/theme.rs`：以統一的動態 `ThemeDef` 表示內建與上傳主題；切換器、預設解析、
   預覽與管理清單不再假設封閉的 `ThemeKey` 集合。
5. 路由與模板：`/app/admin/themes` 的 multipart 上傳、編輯頁、刪除、稽核 action，以及目錄 API
   的 `custom` 旗標。
6. 資產：`/assets/custom-themes/{key}` 依資料庫目前資料列提供
   `URLET_THEMES_DIR/<key>-<hash>.css`，快取一小時；替換 CSS 會改變 URL。
7. 持久化：`URLET_THEMES_DIR` 預設為 `<data dir>/themes`，Docker 建立 `/data/themes`，compose
   掛載獨立的 `urlet-themes` volume。
8. 測試與文件：上傳/管理/刪除、驗證、授權、重載後仍在、稽核紀錄、雙語鏡像與
   Docker/compose/備份說明。

**驗收**：管理員可上傳 CSS 主題、選擇 `auto`／`light`／`dark` 外觀、設為預設、替換樣式表，
並在移轉預設後刪除；成員對所有上傳路由皆回 403；重載後 registry 能從 SQLite 重建目錄；強制
明暗模式可覆寫瀏覽器儲存的偏好；CSS 檔存在於 `URLET_THEMES_DIR`；且
`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 與
`scripts/smoke.sh` 全綠。

---

## P15 — 公開短網址工具 (v1.6)

**任務**

1. `migrations/0015_public_shorten.sql`：`users.is_system` 與其部分唯一索引、保留的訪客帳號
   （`guest@urlet.invalid`、已停用、無法解析的雜湊、無群組）以及十二個公開短網址工具設定
2. `src/db/users.rs`：`is_system` 欄位、`find_system`，並在每個列表、計數與管理寫入加上
   `is_system = 0`
3. `src/services/settings.rs`：`PublicCreateAccess`（`guest`/`user`/`admin`）、`PublicFields`、
   `public_*` 存取子與驗證
4. `src/services/public_urls.rs`：存取檢查、伺服器端欄位強制、每來源位址與共用額度，以及解析
   訪客 actor 的建立流程
5. `src/services/urls.rs`：存取層級同時把關儀表板與已認證 API
6. `src/routes/public_urls.rs` + `src/routes/pages/home.rs`：`POST /`、`POST /api/v1/public/urls`、
   表單／結果檢視，以及永不信任查詢字串的 `?shortened=` 查詢
7. `templates/partials/shorten.html` 與六個導覽首頁模板的 include；當建立權保留給管理員時，
   儀表板顯示提示
8. 管理設定區塊、三份語言目錄、`tests/public_urls.rs`、`scripts/smoke.sh` 與雙語文件

**驗收**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、
`scripts/smoke.sh` 與 `scripts/merge-check.sh` 全綠；升級後公開表單維持關閉，直到管理員開啟。

---

## P16 — 可編輯的訪客目標網址白名單

**目標**：未登入訪客只能縮短明確允許的目標，管理員可編輯內建清單並立即生效。範圍僅含本機白名單，不含可信度 API、黑名單、群組覆寫或轉址時檢查。

**架構／基準**：沿用 SQLite 設定及原子化、具稽核紀錄的管理 API／表單（`docs/SPEC.md` §4.16）。新增單一職責的比對模組，供公開 HTML／JSON 建立共用。既有網域政策管理的是短網址網域，不能限制目標，因此需要修改程式。使用既有依賴樹中的 `url::Url` 解析。無須修改 schema，以既有預設設定機制補齊缺值。

**相容性**：`guest_allowlist_enabled` 預設為 `true`，不覆寫已編輯或刻意清空的清單。啟用且清單為空時拒絕所有訪客目標。檢查先於驗證碼及訪客共用額度。登入者與既有轉址維持原有行為。舊版設定表單省略新控制項時保留新設定。大型管理／設定 handler 僅接線，比對邏輯放在獨立模組。

**任務**

1. 新增 `src/services/guest_allowlist.rs` 與知名網站內建清單，適合時限定內容路徑。支援精確 ASCII DNS 主機名稱與選填、區分大小寫、依路徑區段比對的前綴，不隱含子網域，不接受帳密或非標準連接埠。限制並驗證編輯內容、處理點路徑，限定路徑的規則拒絕有歧義的編碼分隔符。單元測試涵蓋偽裝主機、路徑、空清單及預設值。
2. 在 `src/db/settings.rs`、`src/services/settings.rs` 及 `src/services/public_urls.rs` 接上 `guest_allowlist_enabled`／`guest_allowlist`。測試匿名表單／API 限制、即時修改、關閉／空清單、登入 session／token 豁免及設定重新載入後的持久性。
3. 擴充既有管理設定頁／表單與共用首頁 partial。新增三語說明、專用拒絕訊息及訪客可見的允許目標清單。測試授權、無效更新的原子性、舊表單及所有內建主題。
4. 更新規格、API 文件、本計畫及繁體中文鏡像；擴充 `scripts/smoke.sh`，透過實際 HTTP 驗證預設限制與清單編輯。
5. 執行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`bash scripts/smoke.sh`、`bash scripts/merge-check.sh`；審查後提交並推送 `feat/guest-allowlist`，建立到 `cxyfer/urlet:master` 的 PR，不合併。本次未要求嚴格 TDD，驗證後統一提交完整功能。

**風險／淘汰**：知名網站仍可能包含使用者內容或轉址，內建清單是可編輯政策，不是安全認證。不抓取目標內容。保留既有設定與速率限制來源，不淘汰舊路徑。驗證涵蓋實際 router 與設定重新載入，不僅是獨立比對測試。

---

## 風險與對策

| 風險 | 影響 | 對策 |
|------|------|------|
| 本機無 Docker 引擎（Docker Desktop WSL 整合未啟用） | 無法實際 `docker build`/`compose up` | 以 YAML 解析驗證 compose、以 release 二進位驗證行為、Dockerfile 採標準多階段樣式並在文件中標註未實機驗證 |
| musl 靜態建置若失敗 | 映像檔無法執行 | 保留 `debian-slim` 執行期作為備援方案（文件中說明） |
| SQLite 寫入鎖競爭 | 高併發點擊寫入延遲 | WAL + `busy_timeout` + 非同步寫入 + 彙總表 upsert 合併寫入 |
| Askama 模板與路由不同步 | 執行期 500 | 模板於編譯期檢查；整合測試覆蓋每個頁面 GET |
| 相依套件版本漂移 | 建置失敗 | 提交 `Cargo.lock`；建置階段固定工具鏈映像版本 |

---

## P8 驗收量測結果

| 項目 | 目標（`docs/SPEC.md` §3.3、§5.2） | 實測 |
|------|-----------------------------------|------|
| Release 二進位大小 | ≤ 15 MB | **4.2 MB**（stripped, LTO, `opt-level=z`） |
| 閒置記憶體 RSS | ≤ 30 MB | **6.8 MB**（500 次請求後 6.8 MB） |
| 轉址延遲（P95） | < 10 ms | **0.74 ms**（p50 0.40 ms、p99 1.05 ms、本機 500 次 keep-alive） |
| 冷啟動到可服務 | ≤ 1 秒 | **約 0.2 秒**（含遷移） |
| 容器映像 | ≤ 40 MB | **13.2 MB**（Docker Compose 27.4 實機建置並 `up -d` 通過） |
| 測試 | 全綠 | `cargo test` 187 項、`scripts/smoke.sh` 81 項 |
| 靜態檢查 | 無警告 | `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 通過 |

上表為 v1.0 的歷史紀錄；v1.1 的最新數字（二進位 5.7 MB glibc／5.8 MB musl、映像 14.8 MB、
RSS 8.7 MB、冷啟動約 0.1 秒、完整測試套件）請見 `docs/SPEC.md` §3.3，不要直接引用上表。
