# uwurls 部署指南

## 1. 系統需求

| 項目 | 最低 | 建議 |
|------|------|------|
| CPU | 1 vCPU | 2 vCPU |
| 記憶體 | 128 MB | 256 MB |
| 磁碟 | 100 MB + 資料 | 1 GB |
| Docker | 24.x | 最新版 |
| Docker Compose | 2.24（TLS overlay 需要 `!override`） | 最新版 |

不需要外部資料庫、Redis 或反向代理。單一容器即可對外服務（建議前面加一層 TLS）。

## 2. 快速部署（HTTP）

```bash
git clone <this repo> uwurls && cd uwurls
cp .env.example .env
```

編輯 `.env`：

```env
UWURLS_BASE_URL=https://s.example.com      # 對外網址，會用來產生短網址
UWURLS_PORT=8080                            # 主機對應的連接埠
UWURLS_ADMIN_EMAIL=admin@example.com
UWURLS_ADMIN_PASSWORD=<強密碼>
UWURLS_REGISTRATION=invite                  # 可選：open / invite / closed
```

啟動：

```bash
docker compose up -d --build
docker compose logs -f uwurls
```

驗證：

```bash
curl -fsS http://localhost:8080/health
docker compose exec uwurls /usr/local/bin/uwurls healthcheck
```

## 3. 加上自動 HTTPS（Caddy）

```bash
export UWURLS_DOMAIN=s.example.com
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d --build
```

- 先把 `s.example.com` 的 A/AAAA 記錄指向主機，Caddy 會自動申請與續期憑證。
- overlay 會把 uwurls 的連接埠對外關閉（僅 compose 網路內可見），並自動設定
  `UWURLS_BASE_URL`、`UWURLS_SECURE_COOKIES=true`、`UWURLS_TRUST_PROXY=true`。
- 憑證存放在 `uwurls-caddy-data` volume。

若使用既有的 Nginx/Traefik，請自行反向代理到 `127.0.0.1:8080` 並設定：

```env
UWURLS_BASE_URL=https://s.example.com
UWURLS_SECURE_COOKIES=true
UWURLS_TRUST_PROXY=true      # 只有在你能確保代理會覆寫 X-Forwarded-* 時才開啟
```

## 4. 設定參考

| 變數 | 預設 | 說明 |
|------|------|------|
| `UWURLS_BASE_URL` | `http://localhost:8080` | 對外網址；用於產生短網址、CSRF 同源檢查、Cookie `Secure` 判斷 |
| `UWURLS_BIND` | `0.0.0.0:8080` | 監聽位址 |
| `UWURLS_DATABASE_URL` | `sqlite:///data/uwurls.db?mode=rwc` | SQLite 連線字串 |
| `UWURLS_DATA_DIR` | 資料庫所在目錄 | 自動產生密鑰的存放位置 |
| `UWURLS_SECRET` | 自動產生 | HMAC 密鑰（至少 16 字元，建議 32+；自動產生的為 44 字元 base64）；**更換會讓所有 Session 與解鎖 Cookie 失效** |
| `UWURLS_ADMIN_EMAIL` / `UWURLS_ADMIN_PASSWORD` | — | 啟動時建立或升級管理員 |
| `UWURLS_REGISTRATION` | 讀取設定表 | 覆寫註冊模式：`open` / `invite` / `closed` |
| `UWURLS_SECURE_COOKIES` | 依 `BASE_URL` 判斷 | 強制 Cookie `Secure` |
| `UWURLS_TRUST_PROXY` | `false` | 信任 `X-Forwarded-For` / `X-Forwarded-Proto` |
| `UWURLS_SESSION_TTL_HOURS` | `168` | Session 有效期 |
| `UWURLS_UNLOCK_TTL_MINUTES` | `30` | 短網址解鎖 Cookie 有效期 |
| `UWURLS_DEFAULT_SLUG_LENGTH` | 讀取設定表 | 覆寫自動 slug 長度（4–16） |
| `UWURLS_LOG` | `info` | tracing 過濾器，例如 `uwurls=debug,tower=warn` |
| `UWURLS_LOG_FORMAT` | `compact` | `compact` 或 `json` |
| `UWURLS_HEALTHCHECK_URL` | `http://127.0.0.1:<port>/health` | `uwurls healthcheck` 的目標 |

站台層級設定（站名、註冊模式、配額、預設轉址類型、統計開關、點擊保留天數）
在 **Admin → Settings** 調整，會存進資料庫並即時生效。

## 5. 資料與備份

所有狀態都在 `/data`（volume `uwurls-data`）：

```
/data/uwurls.db         # 主要資料庫
/data/uwurls.db-wal     # write-ahead log
/data/uwurls.db-shm
/data/secret.key        # 未設定 UWURLS_SECRET 時自動產生的密鑰（0600）
```

### 備份

SQLite 處於 WAL 模式，直接 `cp` 主檔可能漏掉還在 WAL 裡的交易。兩種安全做法：

**方法 A（主機有 `sqlite3`，不需停機）**

```bash
VOL=$(docker volume inspect -f '{{ .Mountpoint }}' uwurls-data)
sudo sqlite3 "$VOL/uwurls.db" ".backup '/tmp/uwurls-$(date +%F).db'"
mv /tmp/uwurls-$(date +%F).db ./backups/
```

`.backup` 使用 SQLite 的線上備份 API，會取得一致的快照。

**方法 B（沒有 `sqlite3`）**

```bash
docker compose stop uwurls          # 正常關閉會 checkpoint WAL
docker run --rm -v uwurls-data:/data -v "$PWD":/backup alpine \
  cp /data/uwurls.db "/backup/uwurls-$(date +%F).db"
docker compose start uwurls
```

別忘了 `secret.key`：若你依賴自動產生的密鑰，備份它（或改用 `UWURLS_SECRET` 環境變數），
否則還原後所有 Session 與解鎖 Cookie 都會失效。

### 還原

```bash
docker compose down
docker run --rm -v uwurls-data:/data -v "$PWD":/backup alpine \
  sh -c 'rm -f /data/uwurls.db*; cp /backup/uwurls-YYYY-MM-DD.db /data/uwurls.db; chown 10001:10001 /data/uwurls.db'
docker compose up -d
```

### 升級

```bash
git pull
docker compose up -d --build      # 啟動時自動套用遷移
```

遷移是向前相容的累加檔案（`migrations/`），已套用的檔案有 checksum 保護，
被修改時啟動會直接失敗而不是默默分歧。升級前建議先備份。

## 6. 資源與硬碟用量

| 資料 | 成長速度 |
|------|----------|
| `urls` | 每筆約 300 bytes |
| `clicks` | 每筆約 150 bytes（可用 `click_retention_days` 自動清理） |
| `url_daily_stats` | 每個短網址每天 1 列 |
| `audit_logs` | 每筆約 200 bytes，自動保留 180 天 |
| `sessions` | 過期或撤銷後 1 天內清除 |
| `login_attempts` | 保留 30 天 |

10 萬次點擊約佔 15 MB。若長期不清理，記得設定 `click_retention_days`。

## 7. 反向代理注意事項

- 一定要設定 `UWURLS_BASE_URL` 為使用者實際看到的網址，否則短網址會指向錯誤的網域。
- 只有在代理**保證覆寫** `X-Forwarded-For` / `X-Real-IP` 時才開啟 `UWURLS_TRUST_PROXY`，
  否則客戶端可以偽造 IP 繞過速率限制。程式優先採用 `X-Real-IP`（代理自行設定），
  其次取 `X-Forwarded-For` 最右側的位址，因此附加式代理也安全；
  隨附的 `deploy/Caddyfile` 已設定 `header_up X-Real-IP {remote_host}`。
- 需要傳遞 `Host`、`X-Forwarded-For`、`X-Forwarded-Proto`；Caddy 會自動處理。

## 8. 監控

- 健康檢查：`GET /health`（200 = 正常，503 = 資料庫異常）
- 容器 `HEALTHCHECK` 已內建，使用 `uwurls healthcheck`（映像檔不需要 curl）
- 日誌輸出到 stdout；`UWURLS_LOG_FORMAT=json` 可搭配 Loki/CloudWatch 之類的收集器
- 每個請求都有 `x-request-id` 回應標頭，並以相同 id 寫入日誌
- 管理後台 **Admin → Overview** 可看使用者數、短網址數、點擊總量、資料庫大小

## 9. 疑難排解

| 症狀 | 原因與處理 |
|------|-----------|
| 啟動即失敗並顯示 migration checksum 不符 | `migrations/` 中已套用的檔案被修改；請還原該檔案或改用新的遷移檔 |
| 登入後馬上被登出 | `UWURLS_BASE_URL` 與實際網址不一致，或 `UWURLS_SECURE_COOKIES=true` 卻用 HTTP 存取 |
| 短網址指向 `http://localhost:8080` | 沒設定 `UWURLS_BASE_URL` |
| 容器啟動後 `/health` 回 503 | volume 權限問題：`/data` 必須可寫（容器以 uid 10001 執行） |
| 速率限制誤擋（NAT 後多個使用者） | 拉長限制或改用 API Token；限制是每 IP 計算 |
| 登入回 429 但密碼正確 | 帳號被暫時鎖定（15 分鐘內 30 次失敗）。等待視窗滑過，或 `sqlite3 /data/uwurls.db "DELETE FROM login_attempts"` 立即解除 |
| 忘記管理員密碼後想用 Token 存取 | 重設密碼會撤銷該帳號所有 API Token（安全性設計），請重新建立 |
| 忘記管理員密碼 | 設定 `UWURLS_ADMIN_EMAIL` 為該帳號並重啟；或 `sqlite3` 直接更新，或建立新管理員後停用舊帳號 |
| 短網址已過期想復原 | 到 `/app/urls/{id}` 清除或延長到期時間；資料未被刪除 |
| 刪除短網址後想重用同一個 slug | 刻意不允許（避免舊連結被劫持）；請改用新 slug |

## 10. 安全檢查清單

- [ ] `UWURLS_BASE_URL` 使用 `https://`
- [ ] `UWURLS_ADMIN_PASSWORD` 為強密碼，且登入後於 `/app/account` 更換
- [ ] 設定 `UWURLS_SECRET`（至少 32 個隨機字元）或確保自動產生的 `secret.key`（0600）已備份
- [ ] 對外只暴露 80/443（由 Caddy 或既有代理終結 TLS）
- [ ] 確認 `UWURLS_TRUST_PROXY` 的設定與代理行為一致
- [ ] 註冊模式設為 `invite` 或 `closed`（除非真的要開放註冊）
- [ ] 已規劃 `click_retention_days` 與備份排程
- [ ] 定期 `docker compose pull && docker compose up -d --build` 更新
