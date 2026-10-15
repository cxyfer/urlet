# uwurls — 分階段實作計畫 (Implementation Plan)

> 對應規格：`docs/SPEC.md` v1.0
> 原則：每個階段結束時程式碼必須**可編譯、可測試、可提交**；每個階段一個（或數個）本地 commit。

---

## 里程碑總覽

| 階段 | 名稱 | 產出 | 驗收 | Commit |
|------|------|------|------|--------|
| P0 | Spec 與計畫 | `docs/SPEC.md`、`docs/PLAN.md` | 規格涵蓋所有需求、可追溯 | `📝 docs: add spec and phased implementation plan` |
| P1 | 專案骨架與資料層 | Cargo 專案、config、13 張表 migrations、db 模組、錯誤處理、health、Dockerfile、compose | `cargo run` 可啟動、`/health` 回 200、遷移成功 | `🎉 init: bootstrap uwurls project skeleton` |
| P2 | 認證與 Session | 註冊/登入/登出、Argon2、Session、CSRF、速率限制、admin bootstrap | 整合測試涵蓋註冊登入、未授權 401、CSRF 403 | `✨ feat(auth): ...` |
| P3 | 短網址核心 | slug 服務、URL CRUD、標籤、過期/點擊上限/密碼欄位、配額 | 整合測試涵蓋 CRUD、所有權隔離、slug 驗證 | `✨ feat(urls): ...` |
| P4 | 轉址與解鎖 | redirect handler、點擊記錄、UA 解析、解鎖 Cookie、410/404 頁 | 整合測試涵蓋轉址、密碼流程、過期 410 | `✨ feat(redirect): ...` |
| P5 | Web UI | Askama 模板 + CSS/JS、儀表板、網址詳情、帳號頁 | 手動 + smoke 驗證頁面可操作 | `✨ feat(ui): ...` |
| P6 | 管理後台與 API | 使用者管理、邀請碼、設定、稽核、API Token、`/api/v1` 完整化 | 整合測試涵蓋 admin 授權與 Token | `✨ feat(admin): ...` |
| P7 | 測試與文件 | 整合測試補齊、`scripts/smoke.sh`、README、API.md、DEPLOYMENT.md | `cargo test` 全綠、smoke 全通過 | `🧪 test: ...` / `📝 docs: ...` |
| P8 | 最終驗證與收尾 | clippy/fmt 清零、release 建置、compose 設定驗證、映像檔策略確認 | 驗收清單全數勾選 | `🔖 chore: finalize v1.0.0` |

---

## P1 — 專案骨架與資料層

**任務**

1. `Cargo.toml`：axum 0.8、tokio、sqlx(sqlite, runtime-tokio, migrate)、askama、argon2、sha2、hmac、time、serde、tracing、tower-http、rand/getrandom、base64、subtle、thiserror、anyhow、uuid？（不需要）
2. `[profile.release]`：`lto=true`、`codegen-units=1`、`strip=true`、`opt-level="z"`、`panic="abort"`
3. `src/config.rs`：環境變數解析 + 預設值 + 驗證（BASE_URL、DB 路徑、secret 自動產生）
4. `migrations/0001_init.sql`：§6.3 全部 13 張表 + 索引 + seed settings
5. `src/db/`：pool 建立（WAL/pragmas）、`migrate()`、各表 CRUD 模組骨架
6. `src/error.rs`：`AppError` 列舉 + `IntoResponse`（HTML 與 JSON 雙模式）
7. `src/server.rs` + `src/main.rs`：router、`/health`、`/assets/*`、優雅關閉
8. `Dockerfile`、`docker-compose.yml`、`docker-compose.tls.yml`、`deploy/Caddyfile`、`.env.example`、`.gitignore`
9. `rustfmt.toml`、`clippy` 設定

**驗收**：`cargo build` 成功；啟動後 `curl /health` 回 200 JSON；DB 檔自動建立且 13 張表存在。

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
10. 首個使用者自動成為 admin；`UWURLS_ADMIN_EMAIL/PASSWORD` bootstrap

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

## 風險與對策

| 風險 | 影響 | 對策 |
|------|------|------|
| 本機無 Docker 引擎（Docker Desktop WSL 整合未啟用） | 無法實際 `docker build`/`compose up` | 以 YAML 解析驗證 compose、以 release 二進位驗證行為、Dockerfile 採標準多階段樣式並在文件中標註未實機驗證 |
| musl 靜態建置若失敗 | 映像檔無法執行 | 保留 `debian-slim` 執行期作為備援方案（文件中說明） |
| SQLite 寫入鎖競爭 | 高併發點擊寫入延遲 | WAL + `busy_timeout` + 非同步寫入 + 彙總表 upsert 合併寫入 |
| Askama 模板與路由不同步 | 執行期 500 | 模板於編譯期檢查；整合測試覆蓋每個頁面 GET |
| 相依套件版本漂移 | 建置失敗 | 提交 `Cargo.lock`；建置階段固定工具鏈映像版本 |
