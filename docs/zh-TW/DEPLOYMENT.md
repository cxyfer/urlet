# uwurls 部署指南

[English](../DEPLOYMENT.md) | **繁體中文**

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
UWURLS_BASE_URL=http://localhost:8080      # 對外網址；加上 TLS 後請改為 https://（見 §3）
UWURLS_PORT=8080                            # 主機對應的連接埠（僅 docker compose 讀取）
UWURLS_ADMIN_EMAIL=admin@example.com
UWURLS_ADMIN_PASSWORD=<強密碼>
UWURLS_REGISTRATION=invite                  # 可選：open / invite / closed
```

> `UWURLS_BASE_URL` 的 scheme 必須與使用者實際連線的方式一致：`https://` 會讓 uwurls 把 Session
> Cookie 標記為 `Secure`，因此一邊用純 HTTP 提供服務、一邊把 `UWURLS_BASE_URL` 設成 `https://`，
> 會導致登入後立刻被登出（見 §4、§10）。

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
| `UWURLS_SECURE_COOKIES` | 依 `BASE_URL` 判斷 | 強制 Cookie `Secure`。compose 檔一律會帶入值（預設 `false`），因此在 Docker 環境自行終結 TLS 時請明確設為 `true`；TLS overlay 已自動處理 |
| `UWURLS_TRUST_PROXY` | `false` | 信任 `X-Forwarded-For` / `X-Forwarded-Proto` |
| `UWURLS_SESSION_TTL_HOURS` | `168` | Session 有效期 |
| `UWURLS_UNLOCK_TTL_MINUTES` | `30` | 短網址解鎖 Cookie 有效期 |
| `UWURLS_DEFAULT_SLUG_LENGTH` | 讀取設定表 | 覆寫自動 slug 長度（4–16） |
| `UWURLS_CAPTCHA_PROVIDER` | `none` | 防機器人驗證供應商：`none` / `turnstile` / `recaptcha` |
| `UWURLS_CAPTCHA_SITE_KEY` | 空 | 公開 site key，會渲染進驗證元件 |
| `UWURLS_CAPTCHA_SECRET_KEY` | 空 | 伺服器端驗證用的密鑰（只存在於伺服器端；後台介面不會回顯） |
| `UWURLS_CAPTCHA_ON_REGISTER` | `false` | 註冊表單是否套用防機器人驗證 |
| `UWURLS_CAPTCHA_ON_LOGIN` | `false` | 登入表單是否套用防機器人驗證 |
| `UWURLS_CAPTCHA_ON_CREATE` | `false` | 建立短網址表單是否套用防機器人驗證 |
| `UWURLS_LOG` | `info` | tracing 過濾器，例如 `uwurls=debug,tower=warn` |
| `UWURLS_LOG_FORMAT` | `compact` | `compact` 或 `json` |
| `UWURLS_HEALTHCHECK_URL` | `http://127.0.0.1:<port>/health` | `uwurls healthcheck` 的目標 |

另外兩個變數由 compose 檔讀取，uwurls 二進位檔本身不會讀取：

| 變數 | 預設 | 說明 |
|------|------|------|
| `UWURLS_PORT` | `8080` | `docker-compose.yml` 對外發布的主機連接埠（容器本身一律監聽 8080） |
| `UWURLS_DOMAIN` | — | `docker-compose.tls.yml` overlay 申請憑證用的網域；該 overlay 必填 |

站台層級設定（站名、註冊模式、配額、預設轉址類型、統計開關、點擊保留天數）
在 **Admin → Settings** 調整，會存進資料庫並即時生效。
非空的環境變數一律覆蓋設定表中的值。

## 5. 防機器人驗證（CAPTCHA）

uwurls 可以要求三個會產生資料的公開表單通過防機器人驗證：網頁註冊表單、網頁登入表單與
網頁建立短網址表單。JSON API 的 `POST /api/v1/auth/register` 與 `POST /api/v1/auth/login`
也涵蓋在內，兩者以選填欄位 `captcha_token` 接收驗證元件的權杖。`POST /api/v1/urls` 同樣在
範圍內，但僅限 Session Cookie 呼叫（受建立短網址開關約束）；帶 `Authorization: Bearer uw_...`
的呼叫維持豁免，自動化流程不受影響。預設關閉。

### 支援的供應商

| 供應商 | 值 | 說明 |
|--------|----|------|
| 關閉 | `none` | 預設。不渲染驗證元件，也不做任何驗證 |
| Cloudflare Turnstile | `turnstile` | 注重隱私、有免費額度，通常使用者無感 |
| Google reCAPTCHA v2 | `recaptcha` | 「我不是機器人」核取方塊 |

### 環境變數

| 變數 | 預設 | 說明 |
|------|------|------|
| `UWURLS_CAPTCHA_PROVIDER` | `none` | `none`、`turnstile` 或 `recaptcha` |
| `UWURLS_CAPTCHA_SITE_KEY` | 空 | 公開 site key |
| `UWURLS_CAPTCHA_SECRET_KEY` | 空 | 密鑰，只存在於伺服器端 |
| `UWURLS_CAPTCHA_ON_REGISTER` | `false` | 驗證註冊表單 |
| `UWURLS_CAPTCHA_ON_LOGIN` | `false` | 驗證登入表單 |
| `UWURLS_CAPTCHA_ON_CREATE` | `false` | 驗證建立短網址表單 |

四個條件**全部**成立時，該表單才會啟用驗證：provider 不是 `none`、site key 非空、
secret key 非空，以及對應的 `ON_*` 開關（`UWURLS_CAPTCHA_ON_REGISTER`、
`UWURLS_CAPTCHA_ON_LOGIN` 或 `UWURLS_CAPTCHA_ON_CREATE`）為 `true`。
只設定 provider 而沒有金鑰，或只有金鑰而沒有 provider，都會讓驗證維持關閉。

驗證失敗一律**拒絕**（fail closed）：token 缺少、格式錯誤、逾期或已使用過，
以及逾時或供應商回應非 2xx，都會拒絕該次提交。網頁表單會顯示驗證錯誤訊息；
對應開關啟用時，`POST /api/v1/auth/register`、`POST /api/v1/auth/login`，以及使用
Session Cookie 的 `POST /api/v1/urls`，都會回 `400` 且 `error.code = "captcha_failed"`。

### 取得金鑰

**Cloudflare Turnstile**

1. 登入 Cloudflare 儀表板，開啟 **Turnstile**。
2. 選擇 **Add widget**，命名後加入使用者實際會連到的網域
   （例如 `s.example.com`；想在本機測試可一併加入 `localhost`）。
3. 選擇元件模式（Managed 是不錯的預設值）並建立。
4. 複製 **Site Key** 與 **Secret Key**。

**Google reCAPTCHA v2**

1. 開啟 reCAPTCHA 管理主控台並新增網站。
2. 選擇 **reCAPTCHA v2** → **「我不是機器人」核取方塊**。
3. 加入你的實例網域。
4. 複製 **Site key** 與 **Secret key**。

### 設定方式

在 `.env`（docker compose）中：

```env
UWURLS_CAPTCHA_PROVIDER=turnstile
UWURLS_CAPTCHA_SITE_KEY=0x4AAAAAAA...
UWURLS_CAPTCHA_SECRET_KEY=0x4AAAAAAA...
UWURLS_CAPTCHA_ON_REGISTER=true
UWURLS_CAPTCHA_ON_LOGIN=true
UWURLS_CAPTCHA_ON_CREATE=true
```

隨附的 `docker-compose.yml` 會把六個 `UWURLS_CAPTCHA_*` 變數直接傳進容器，
因此只要在 `.env` 設定即可，不需要修改 compose 檔。接著執行
`docker compose up -d` 重建容器。

同樣的值也可以在管理後台 **Admin → Settings** 輸入，不需要重啟即生效。
密鑰在後台是唯寫的：不會回顯已儲存的值，留白則維持原值。
請記得非空的環境變數會覆蓋後台設定，因此每個值只在一個地方設定。

### 反向代理後面的 `remoteip`

伺服器端驗證會把訪客 IP 以 `remoteip` 傳給供應商，讓供應商做風險評分。
uwurls 解析這個位址的方式與速率限制完全相同：

- `UWURLS_TRUST_PROXY=false`（預設）——採用直接連線的對方位址；前面有代理時，
  那就是代理自己的位址。此時所有訪客在供應商眼中都是同一個用戶端，
  評分只依賴 token 本身的訊號。
- `UWURLS_TRUST_PROXY=true`——優先採用 `X-Real-IP`，其次取 `X-Forwarded-For` 最右側的位址。
  隨附的 `deploy/Caddyfile` 已設定 `header_up X-Real-IP {remote_host}`，
  因此 Caddy 會傳遞真實的訪客位址。
- 無法判斷位址時會省略 `remoteip`；兩家供應商的 API 都把它視為選填，驗證仍可正常運作。

只有在代理**保證覆寫** `X-Real-IP` / `X-Forwarded-For` 時才開啟 `UWURLS_TRUST_PROXY`，
否則客戶端可以偽造 IP，同時繞過速率限制。

## 6. 資料與備份

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

## 7. 資源與硬碟用量

| 資料 | 成長速度 |
|------|----------|
| `urls` | 每筆約 300 bytes |
| `clicks` | 每筆約 150 bytes（可用 `click_retention_days` 自動清理） |
| `url_daily_stats` | 每個短網址每天 1 列 |
| `audit_logs` | 每筆約 200 bytes，自動保留 180 天 |
| `sessions` | 過期或撤銷後 1 天內清除 |
| `login_attempts` | 保留 30 天 |

10 萬次點擊約佔 15 MB。若長期不清理，記得設定 `click_retention_days`。

## 8. 反向代理注意事項

- 一定要設定 `UWURLS_BASE_URL` 為使用者實際看到的網址，否則短網址會指向錯誤的網域。
- 只有在代理**保證覆寫** `X-Forwarded-For` / `X-Real-IP` 時才開啟 `UWURLS_TRUST_PROXY`，
  否則客戶端可以偽造 IP 繞過速率限制。程式優先採用 `X-Real-IP`（代理自行設定），
  其次取 `X-Forwarded-For` 最右側的位址，因此附加式代理也安全；
  隨附的 `deploy/Caddyfile` 已設定 `header_up X-Real-IP {remote_host}`。
- 需要傳遞 `Host`、`X-Forwarded-For`、`X-Forwarded-Proto`；Caddy 會自動處理。

## 9. 監控

- 健康檢查：`GET /health`（200 = 正常，503 = 資料庫異常）
- 容器 `HEALTHCHECK` 已內建，使用 `uwurls healthcheck`（映像檔不需要 curl）
- 日誌輸出到 stdout；`UWURLS_LOG_FORMAT=json` 可搭配 Loki/CloudWatch 之類的收集器
- 每個請求都有 `x-request-id` 回應標頭，並以相同 id 寫入日誌
- 管理後台 **Admin → Overview** 可看使用者數、短網址數、點擊總量、資料庫大小

## 10. 疑難排解

| 症狀 | 原因與處理 |
|------|-----------|
| 啟動即失敗並顯示 migration checksum 不符 | `migrations/` 中已套用的檔案被修改；請還原該檔案或改用新的遷移檔 |
| 登入後馬上被登出 | `UWURLS_BASE_URL` 與實際網址不一致，或 `UWURLS_SECURE_COOKIES=true` 卻用 HTTP 存取 |
| 短網址指向 `http://localhost:8080` | 沒設定 `UWURLS_BASE_URL` |
| 容器啟動後 `/health` 回 503 | volume 權限問題：`/data` 必須可寫（容器以 uid 10001 執行） |
| 速率限制誤擋（NAT 後多個使用者） | 拉長限制或改用 API Token；限制是每 IP 計算 |
| 登入回 429 但密碼正確 | 帳號被暫時鎖定（15 分鐘內 30 次失敗）。等待視窗滑過，或 `sqlite3 /data/uwurls.db "DELETE FROM login_attempts"` 立即解除 |
| 驗證元件完全沒出現 | 四個條件都要成立：provider 不是 `none`、兩把金鑰都非空、對應的 `ON_*` 為 `true` |
| 每次提交都回 `captcha_failed` | 密鑰不屬於目前設定的供應商、元件的網域清單沒有包含使用者實際連到的網域，或 API 用戶端忘了以 `captcha_token` 帶上驗證元件的權杖 |
| 忘記管理員密碼後想用 Token 存取 | 重設密碼會撤銷該帳號所有 API Token（安全性設計），請重新建立 |
| 忘記管理員密碼 | 啟動時的 bootstrap 必須 `UWURLS_ADMIN_EMAIL` 與 `UWURLS_ADMIN_PASSWORD` **兩者皆非空**才會執行；對已存在的帳號只會提權，**不會**變更密碼（密碼僅在帳號不存在時使用）。救援方式：把 `UWURLS_ADMIN_EMAIL` 指向另一個你仍登得入的帳號（`UWURLS_ADMIN_PASSWORD` 隨便填一個非空值即可讓 bootstrap 執行），重啟後以該帳號登入；或直接用 `sqlite3` 覆寫密碼雜湊 |
| 短網址已過期想復原 | 到 `/app/urls/{id}` 清除或延長到期時間；資料未被刪除 |
| 刪除短網址後想重用同一個 slug | 刻意不允許（避免舊連結被劫持）；請改用新 slug |

## 11. 安全檢查清單

- [ ] `UWURLS_BASE_URL` 使用 `https://`
- [ ] `UWURLS_ADMIN_PASSWORD` 為強密碼，且登入後於 `/app/account` 更換
- [ ] 設定 `UWURLS_SECRET`（至少 32 個隨機字元）或確保自動產生的 `secret.key`（0600）已備份
- [ ] 對外只暴露 80/443（由 Caddy 或既有代理終結 TLS）
- [ ] 確認 `UWURLS_TRUST_PROXY` 的設定與代理行為一致
- [ ] 註冊模式設為 `invite` 或 `closed`（除非真的要開放註冊）
- [ ] 已規劃 `click_retention_days` 與備份排程
- [ ] 定期 `docker compose pull && docker compose up -d --build` 更新
- [ ] 若實例可從公網連線，已為註冊與登入流程啟用防機器人驗證
