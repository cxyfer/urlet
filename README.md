# uwurls

**輕量化、自託管的多用戶短網址系統。** 單一 Rust 二進位檔 + 單一 SQLite 檔案，
一個 `docker compose up -d` 就能上線，不需要外部資料庫、快取服務或前端建置流程。

![Dashboard](docs/screenshots/dashboard.png)

## 功能

| 功能 | 說明 |
|------|------|
| 👥 多用戶 | 每位使用者獨立管理自己的短網址；跨帳號存取一律 404 |
| 🔗 短網址管理 | 自訂或自動產生 slug、標題、備註、標籤、啟用/停用、軟刪除 |
| 🔒 密碼保護 | 短網址層級密碼（Argon2id），解鎖後以簽章 Cookie 記住 30 分鐘 |
| ⏳ 過期機制 | 絕對到期時間與點擊次數上限，過期回 410 Gone |
| 📊 點擊分析 | 每日趨勢、來源網域、裝置/瀏覽器/作業系統分佈、最近點擊 |
| 🧩 標籤 | 依標籤篩選與分類，附使用次數 |
| 🔑 API Token | `Authorization: Bearer uw_...`，可設到期與撤銷 |
| 🛠 管理後台 | 使用者管理、全站短網址、邀請碼、站台設定、稽核日誌 |
| 🐳 部署 | 單一容器 + named volume；附 Caddy 自動 HTTPS overlay |
| 🪶 輕量 | 二進位 4.2 MB、映像 13.2 MB、RSS 7 MB、轉址 P95 0.74 ms |

## 快速開始

### Docker Compose（推薦）

```bash
git clone <this repo> uwurls && cd uwurls
cp .env.example .env      # 至少設定 UWURLS_BASE_URL 與管理員帳密
docker compose up -d --build
```

打開 `http://localhost:8080`，用 `.env` 裡的 `UWURLS_ADMIN_EMAIL` / `UWURLS_ADMIN_PASSWORD`
登入（伺服器啟動時會自動建立該管理員）。

想讓 Caddy 自動申請 HTTPS 憑證：

```bash
export UWURLS_DOMAIN=s.example.com
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d --build
```

> 需要 Docker Compose ≥ 2.24（overlay 使用 `!override` 標籤）。

### 直接執行二進位檔

```bash
cargo build --release
UWURLS_BASE_URL=http://localhost:8080 \
UWURLS_DATA_DIR=./data \
UWURLS_ADMIN_EMAIL=admin@example.com \
UWURLS_ADMIN_PASSWORD=change-me-please \
./target/release/uwurls serve
```

### 命令列

```
uwurls serve                 # 啟動伺服器（預設）
uwurls migrate               # 套用資料庫遷移後結束
uwurls healthcheck [URL]     # 健康檢查（容器 HEALTHCHECK 使用，無需 curl）
uwurls version | help
```

## 首次使用

1. 若設定了 `UWURLS_ADMIN_EMAIL` / `UWURLS_ADMIN_PASSWORD`，啟動時會建立管理員。
2. 若資料庫中**沒有任何帳號**，第一位註冊的使用者會自動成為管理員。
3. 之後可在 **Admin → Invites** 產生邀請碼，並在 **Admin → Settings** 把註冊模式改為
   `invite`（僅邀請碼）或 `closed`（關閉註冊）。

## 畫面

| 短網址詳情與統計 | 管理後台 |
|---|---|
| ![URL detail](docs/screenshots/url-detail.png) | ![Admin](docs/screenshots/admin-overview.png) |

| 密碼保護頁 | 已過期連結（可一鍵延長） |
|---|---|
| ![Password page](docs/screenshots/password-page.png) | ![Expired link](docs/screenshots/expired-link.png) |

深色模式：![Dark mode](docs/screenshots/dark.png)

## 技術選擇

| 層級 | 選擇 | 為什麼 |
|------|------|--------|
| HTTP | Axum 0.8 + Tokio | 型別安全、生態成熟、單一二進位 |
| 資料庫 | SQLite（sqlx，WAL） | 免外部服務、單檔備份，自託管規模足夠 |
| 遷移 | 內嵌 SQL + checksum 驗證 | 啟動自動遷移，二進位檔自帶 schema |
| 介面 | Askama SSR + 手寫 CSS/JS | 沒有 Node/Vite 建置步驟，映像檔極小 |
| 認證 | 伺服器端 Session + API Token | 可即時撤銷，不必引入 JWT 相依 |
| 密碼 | Argon2id（19 MiB, t=2, p=1） | OWASP 建議參數 |

**刻意不做**：OAuth、使用者群組權限矩陣、GeoIP、關鍵字黑名單、Webhook、郵件寄送。
資料表與架構已為這些功能預留空間（見 `docs/SPEC.md` §12）。

## 專案結構

```
uwurls/
├── Cargo.toml
├── Dockerfile                  # 多階段建置（rust:alpine → alpine）
├── docker-compose.yml          # 單一容器 + named volume
├── docker-compose.tls.yml      # 可選：Caddy 自動 HTTPS
├── deploy/Caddyfile
├── migrations/                 # 內嵌 SQL 遷移
├── templates/                  # Askama 模板（編譯期檢查）
├── scripts/
│   ├── smoke.sh                # 端對端驗收測試（curl）
│   ├── demo.sh                 # 產生示範資料並截圖
│   └── screenshot.py           # Playwright 截圖腳本
├── src/
│   ├── assets/                 # 內嵌 CSS/JS/favicon
│   ├── db/                     # 所有 SQL（每張表一個模組）
│   ├── middleware/             # 認證、CSRF、安全標頭、request id
│   ├── routes/                 # HTTP：api_*、pages/*、redirect、health
│   ├── services/               # 業務邏輯：auth、urls、slug、click、admin...
│   ├── config.rs  error.rs  state.rs  server.rs
│   └── main.rs                 # CLI 入口
└── docs/{SPEC.md,PLAN.md,API.md,DEPLOYMENT.md}
```

## 開發

```bash
cargo test                 # 單元 + 整合測試（187 個）
cargo clippy --all-targets -- -D warnings
cargo fmt --check
bash scripts/smoke.sh      # 對真實二進位檔跑端對端驗收（81 項）
bash scripts/demo.sh       # 產生示範站並輸出截圖到 .tmp/demo/shots
```

需要 Rust 1.80 以上。整合測試使用獨立的臨時 SQLite，不會碰到你的開發資料。

## 文件

- [`docs/SPEC.md`](docs/SPEC.md) — 規格書：需求、資料模型、API、非功能需求
- [`docs/PLAN.md`](docs/PLAN.md) — 分階段實作計畫與驗收標準
- [`docs/API.md`](docs/API.md) — JSON API 參考
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) — 部署、設定、備份、升級、疑難排解

## 安全性

- Argon2id 密碼雜湊；Session / API Token / 邀請碼只存雜湊值
- Cookie：`HttpOnly`、`SameSite=Lax`、可強制 `Secure`
- CSRF：Cookie 認證的狀態變更請求檢查同源；表單類請求一律檢查
- 安全標頭：CSP（`default-src 'self'`）、`X-Content-Type-Options`、
  `X-Frame-Options`、`Referrer-Policy`、HTTPS 時加 HSTS
- 開放轉址防護：目標僅允許 `http`/`https`，`javascript:`/`data:`/`file:` 一律拒絕
- 隱私：不存原始 IP，只存 HMAC 後的 `ip_hash` 與每日輪替的 `visitor_hash`
- 速率限制：登入（每 IP 與每帳號兩層）、註冊、解鎖、建立短網址、改密碼
- 帳號鎖定：同一帳號 15 分鐘內 30 次失敗即暫時拒絕登入（持久化，重啟不重置）
- 反向代理：優先信任代理設定的 `X-Real-IP`，其次取 `X-Forwarded-For` 最右側
- 密碼雜湊在阻塞執行緒池中進行，並限制同時最多 4 個 Argon2 計算
- 稽核日誌：登入、註冊、短網址異動、Token、管理操作、設定變更

## 授權

MIT，見 [`LICENSE`](LICENSE)。
