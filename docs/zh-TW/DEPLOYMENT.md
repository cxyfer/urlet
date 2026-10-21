# urlet 部署指南

[English](../DEPLOYMENT.md) | **繁體中文**

## 1. 系統需求

| 項目 | 最低 | 建議 |
|------|------|------|
| CPU | 1 vCPU | 2 vCPU |
| 記憶體 | 128 MB | 256 MB |
| 磁碟 | 100 MB + 資料 | 1 GB |
| Docker | 24.x | 最新版 |
| Docker Compose | 2.24（overlay 使用 `!override` / `!reset`） | 最新版 |

不需要外部資料庫、Redis 或反向代理。單一容器即可對外服務（建議前面加一層 TLS）。

## 2. 快速部署（HTTP）

```bash
git clone <this repo> urlet && cd urlet
cp .env.example .env
```

編輯 `.env`：

```env
URLET_BASE_URL=http://localhost:8080      # 對外網址；加上 TLS 後請改為 https://（見 §3）
URLET_PORT=8080                            # 主機對應的連接埠（僅 docker compose 讀取）
URLET_ADMIN_EMAIL=admin@example.com
URLET_ADMIN_PASSWORD=<強密碼>
URLET_REGISTRATION=invite                  # 可選：open / invite / closed
```

> `URLET_BASE_URL` 的 scheme 必須與使用者實際連線的方式一致：`https://` 會讓 urlet 把 Session
> Cookie 標記為 `Secure`，因此一邊用純 HTTP 提供服務、一邊把 `URLET_BASE_URL` 設成 `https://`，
> 會導致登入後立刻被登出（見 §4、§10）。

啟動：

```bash
docker compose up -d --build
docker compose logs -f urlet
```

驗證：

```bash
curl -fsS http://localhost:8080/health
docker compose exec urlet /usr/local/bin/urlet healthcheck
```

### 2.1 使用預先建置的映像

每次推送 `v*` 標籤都會發布多架構映像（`linux/amd64`、`linux/arm64`）到 GHCR。若不想在本機
建置，可直接執行已發布的版本：

```bash
export URLET_IMAGE=ghcr.io/cxyfer/urlet:v1.2.0   # 固定要使用的版本
docker compose -f docker-compose.yml -f docker-compose.image.yml up -d
```

- `URLET_IMAGE` 預設為 `ghcr.io/cxyfer/urlet:latest`；正式環境請固定精確版本，不要長期
  追蹤 `latest`。若使用 `latest`，請在 `up -d` 前先執行 `docker compose -f docker-compose.yml
  -f docker-compose.image.yml pull`，才會取得較新的版本。
- overlay 會移除 `build` 設定，因此 compose 只會拉取映像，不會退回本機 Rust 建置。
- 套件預設為**私有**。請先登入（`docker login ghcr.io`，權杖需具備 `read:packages`），或在
  *Package settings → Change visibility* 將套件改為公開。
- 映像以 uid/gid `10001` 執行，與本機建置的映像相同，因此 `/data` volume 的權限設定不變。

## 3. 加上自動 HTTPS（Caddy）

```bash
export URLET_DOMAIN=s.example.com
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d --build
```

- 先把 `s.example.com` 的 A/AAAA 記錄指向主機，Caddy 會自動申請與續期憑證。
- overlay 會把 urlet 的連接埠對外關閉（僅 compose 網路內可見），並自動設定
  `URLET_BASE_URL`、`URLET_SECURE_COOKIES=true`、`URLET_TRUST_PROXY=true`。
- 憑證存放在 `urlet-caddy-data` volume。

若使用既有的 Nginx/Traefik，請自行反向代理到 `127.0.0.1:8080` 並設定：

```env
URLET_BASE_URL=https://s.example.com
URLET_SECURE_COOKIES=true
URLET_TRUST_PROXY=true      # 只有在你能確保代理會覆寫 X-Forwarded-* 時才開啟
```

## 4. 設定參考

| 變數 | 預設 | 說明 |
|------|------|------|
| `URLET_BASE_URL` | `http://localhost:8080` | 對外網址；用於產生短網址、CSRF 同源檢查、Cookie `Secure` 判斷。Host 必須是 ASCII（IDN 請用 punycode）；IPv6 literal 與底線服務名稱僅限 canonical host |
| `URLET_DOMAINS` | 空 | 額外網域（僅轉址），以逗號分隔。第一次啟動時插入為 active；不會覆寫既有狀態。所有網域必須和 `URLET_BASE_URL` 使用相同 http/https scheme，且需要 DNS 與 TLS |
| `URLET_BIND` | `0.0.0.0:8080` | 監聽位址 |
| `URLET_DATABASE_URL` | `sqlite:///data/urlet.db?mode=rwc` | SQLite 連線字串 |
| `URLET_DATA_DIR` | 資料庫所在目錄 | 自動產生密鑰的存放位置 |
| `URLET_SECRET` | 自動產生 | HMAC 密鑰（至少 16 字元，建議 32+；自動產生的為 44 字元 base64）；**更換會讓所有 Session 與解鎖 Cookie 失效，既有邀請碼也無法再顯示（但仍可兌換）** |
| `URLET_ADMIN_EMAIL` / `URLET_ADMIN_PASSWORD` | — | 啟動時建立或升級管理員 |
| `URLET_REGISTRATION` | 讀取設定表 | 覆寫註冊模式：`open` / `invite` / `closed` |
| `URLET_SECURE_COOKIES` | 依 `BASE_URL` 判斷 | 強制 Cookie `Secure`。compose 檔一律會帶入值（預設 `false`），因此在 Docker 環境自行終結 TLS 時請明確設為 `true`；TLS overlay 已自動處理 |
| `URLET_TRUST_PROXY` | `false` | 信任 `X-Forwarded-For` / `X-Forwarded-Proto` |
| `URLET_SESSION_TTL_HOURS` | `168` | Session 有效期 |
| `URLET_UNLOCK_TTL_MINUTES` | `30` | 短網址解鎖 Cookie 有效期 |
| `URLET_DEFAULT_SLUG_LENGTH` | 讀取設定表 | 覆寫自動 slug 長度（4–16） |
| `URLET_CAPTCHA_PROVIDER` | `none` | 防機器人驗證供應商：`none` / `turnstile` / `recaptcha` |
| `URLET_CAPTCHA_SITE_KEY` | 空 | 公開 site key，會渲染進驗證元件 |
| `URLET_CAPTCHA_SECRET_KEY` | 空 | 伺服器端驗證用的密鑰（只存在於伺服器端；後台介面不會回顯） |
| `URLET_CAPTCHA_ON_REGISTER` | `false` | 註冊表單是否套用防機器人驗證 |
| `URLET_CAPTCHA_ON_LOGIN` | `false` | 登入表單是否套用防機器人驗證 |
| `URLET_CAPTCHA_ON_CREATE` | `false` | 建立短網址表單是否套用防機器人驗證 |
| `URLET_CAPTCHA_ALLOWED_HOSTNAMES` | 空 | 供應商 `hostname` 回應中額外接受的主機名稱，以逗號分隔（最多 10 項）；`URLET_BASE_URL` 的主機名稱一律接受 |
| `URLET_LOG` | `info` | tracing 過濾器，例如 `urlet=debug,tower=warn` |
| `URLET_LOG_FORMAT` | `compact` | `compact` 或 `json` |
| `URLET_HEALTHCHECK_URL` | `http://127.0.0.1:<port>/health` | `urlet healthcheck` 的目標 |

### 多網域部署

> **從 1.2 或更舊版本升級：** 請先備份 SQLite 資料庫。多網域 migration
> （`0003_multi_domain`）會重建 `urls` 表，把 slug 命名空間改為以網域為單位；migration 已測試
> 會保留 clicks、tags 與每日統計，但備份是唯一的回滾方式。

Canonical host 保留完整應用（`/`、`/app`、`/api/v1`）。額外網域只做轉址：`/{slug}` 與
`/{slug}/unlock` 可用，`/` 顯示簡單品牌頁，`/app` 導回 canonical，`/api` 回 404。每個網域有
自己的 slug 命名空間。

- 用 `URLET_DOMAINS=go.example.com,links.example.com` 預先加入，或從 `/app/admin/domains` 新增。
- 後台新增的網域是 `pending`。先把 DNS 指向此實例、把 hostname 加進 Caddy site label（或你的
  反向代理），確認 `https://<domain>/health` 可用後，再按「啟用」。
- Canonical 網域不可停用或刪除；目前的預設網域必須先換掉，才能停用或刪除。若 `URLET_BASE_URL`
  改變，下次啟動會把舊 canonical 網域降級為一般管理網域（既有連結繼續可用），並在舊網域原本是
  預設時把預設轉移到新 canonical 網域。
- 所有網域必須和 `URLET_BASE_URL` 使用相同 scheme，不要混用 http 與 https。
- 反向代理必須保留原始的 `Host` header；本版本不信任 `X-Forwarded-Host`。
- `URLET_DOMAINS` 只補缺少：在後台停用 seed 網域後，重啟不會被改回 active。在有設定
  `URLET_DOMAINS` 的前提下，把 hostname 從環境變數移除後，下次啟動會 detach 成一般管理網域
  （保留狀態、連結與預設），不會自動刪除。Docker Compose 一律會傳入此變數，因此空的
  `URLET_DOMAINS` 會被視為明確的空清單。

另外三個變數由 compose 檔讀取，urlet 二進位檔本身不會讀取：

| 變數 | 預設 | 說明 |
|------|------|------|
| `URLET_PORT` | `8080` | `docker-compose.yml` 對外發布的主機連接埠（容器本身一律監聽 8080） |
| `URLET_DOMAIN` | — | `docker-compose.tls.yml` overlay 申請憑證用的網域；該 overlay 必填 |
| `URLET_IMAGE` | `ghcr.io/cxyfer/urlet:latest` | `docker-compose.image.yml` overlay 使用的映像（僅在該 overlay 生效） |

站台層級設定（站名、註冊模式、配額、預設轉址類型、統計開關、點擊保留天數）
在 **Admin → Settings** 調整，會存進資料庫並即時生效。
非空的環境變數一律覆蓋設定表中的值。

## 5. 防機器人驗證（CAPTCHA）

urlet 可以要求三個會產生資料的公開表單通過防機器人驗證：網頁註冊表單、網頁登入表單與
網頁建立短網址表單。JSON API 的 `POST /api/v1/auth/register` 與 `POST /api/v1/auth/login`
也涵蓋在內，兩者以選填欄位 `captcha_token` 接收驗證元件的權杖。`POST /api/v1/urls` 同樣在
範圍內，但僅限 Session Cookie 呼叫（受建立短網址開關約束）；帶 `Authorization: Bearer ur_...`
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
| `URLET_CAPTCHA_PROVIDER` | `none` | `none`、`turnstile` 或 `recaptcha` |
| `URLET_CAPTCHA_SITE_KEY` | 空 | 公開 site key |
| `URLET_CAPTCHA_SECRET_KEY` | 空 | 密鑰，只存在於伺服器端 |
| `URLET_CAPTCHA_ON_REGISTER` | `false` | 驗證註冊表單 |
| `URLET_CAPTCHA_ON_LOGIN` | `false` | 驗證登入表單 |
| `URLET_CAPTCHA_ON_CREATE` | `false` | 驗證建立短網址表單 |
| `URLET_CAPTCHA_ALLOWED_HOSTNAMES` | 空 | 額外接受其 token 的主機名稱，以逗號分隔（最多 10 項） |

四個條件**全部**成立時，該表單才會啟用驗證：provider 不是 `none`、site key 非空、
secret key 非空，以及對應的 `ON_*` 開關（`URLET_CAPTCHA_ON_REGISTER`、
`URLET_CAPTCHA_ON_LOGIN` 或 `URLET_CAPTCHA_ON_CREATE`）為 `true`。
只設定 provider 而沒有金鑰，或只有金鑰而沒有 provider，都會讓驗證維持關閉。

驗證失敗一律**拒絕**（fail closed）：token 缺少、格式錯誤、逾期或已使用過，
以及逾時或供應商回應非 2xx，都會拒絕該次提交。網頁表單會顯示驗證錯誤訊息；
對應開關啟用時，`POST /api/v1/auth/register`、`POST /api/v1/auth/login`，以及使用
Session Cookie 的 `POST /api/v1/urls`，都會回 `400` 且 `error.code = "captcha_failed"`。

### Token 的主機名稱（多主機名稱部署）

兩家供應商都會在 `siteverify` 回應中回報 token 是在哪個主機名稱解出的，urlet 會加以驗證：
token 必須是在**本站**解出的。因此回報的 hostname 必須等於 `URLET_BASE_URL` 的主機名稱，
或是 `URLET_CAPTCHA_ALLOWED_HOSTNAMES` 中列出的其中一個。回應未帶 `hostname` 或該欄位為空字串
時會予以接受 —— 並非每個供應商或元件設定都會回傳這個欄位。

`URLET_CAPTCHA_ALLOWED_HOSTNAMES` 接受以逗號分隔的裸主機名稱清單：每一項會 trim、轉小寫、
丟棄空項，最多保留 **10** 項，每項 1–253 字元；內容不得含空白、`/`、`:` 或 `@`。
留空代表「只接受 `URLET_BASE_URL` 的主機名稱」，這也是常見單一網域部署的正確設定。

> **若你的實例以多個主機名稱提供表單**（例如短網址網域與應用網域不同，或另有別名），
> 就必須把那些主機名稱全部列在這裡。若 token 解出的主機名稱既不是 `URLET_BASE_URL` 的主機
> 名稱、也不在清單中，就會被拒絕，症狀是「使用者填完驗證碼仍無法註冊或登入」。
> 發生不符時 urlet 會記一筆 `WARN`，內容包含回報的 hostname 與目前接受的候選清單，
> 是找出漏填項目最快的方法。

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
URLET_CAPTCHA_PROVIDER=turnstile
URLET_CAPTCHA_SITE_KEY=0x4AAAAAAA...
URLET_CAPTCHA_SECRET_KEY=0x4AAAAAAA...
URLET_CAPTCHA_ON_REGISTER=true
URLET_CAPTCHA_ON_LOGIN=true
URLET_CAPTCHA_ON_CREATE=true
# 可選：額外接受其 token 的主機名稱（以逗號分隔）
# URLET_CAPTCHA_ALLOWED_HOSTNAMES=www.example.com,alt.example.com
```

隨附的 `docker-compose.yml` 會把七個 `URLET_CAPTCHA_*` 變數直接傳進容器，
因此只要在 `.env` 設定即可，不需要修改 compose 檔。接著執行
`docker compose up -d` 重建容器。

同樣的值也可以在管理後台 **Admin → Settings** 輸入，不需要重啟即生效。
密鑰在後台是唯寫的：不會回顯已儲存的值，留白則維持原值。
請記得非空的環境變數會覆蓋後台設定，因此每個值只在一個地方設定。

### 反向代理後面的 `remoteip`

伺服器端驗證會把訪客 IP 以 `remoteip` 傳給供應商，讓供應商做風險評分。
urlet 解析這個位址的方式與速率限制完全相同：

- `URLET_TRUST_PROXY=false`（預設）——採用直接連線的對方位址；前面有代理時，
  那就是代理自己的位址。此時所有訪客在供應商眼中都是同一個用戶端，
  評分只依賴 token 本身的訊號。
- `URLET_TRUST_PROXY=true`——優先採用 `X-Real-IP`，其次取 `X-Forwarded-For` 最右側的位址。
  隨附的 `deploy/Caddyfile` 已設定 `header_up X-Real-IP {remote_host}`，
  因此 Caddy 會傳遞真實的訪客位址。
- 無法判斷位址時會省略 `remoteip`；兩家供應商的 API 都把它視為選填，驗證仍可正常運作。

只有在代理**保證覆寫** `X-Real-IP` / `X-Forwarded-For` 時才開啟 `URLET_TRUST_PROXY`，
否則客戶端可以偽造 IP，同時繞過速率限制。

## 6. 資料與備份

所有狀態都在 `/data`（volume `urlet-data`）：

```
/data/urlet.db         # 主要資料庫
/data/urlet.db-wal     # write-ahead log
/data/urlet.db-shm
/data/secret.key        # 未設定 URLET_SECRET 時自動產生的密鑰（0600）
```

### 備份

SQLite 處於 WAL 模式，直接 `cp` 主檔可能漏掉還在 WAL 裡的交易。兩種安全做法：

**方法 A（主機有 `sqlite3`，不需停機）**

```bash
VOL=$(docker volume inspect -f '{{ .Mountpoint }}' urlet-data)
sudo sqlite3 "$VOL/urlet.db" ".backup '/tmp/urlet-$(date +%F).db'"
mv /tmp/urlet-$(date +%F).db ./backups/
```

`.backup` 使用 SQLite 的線上備份 API，會取得一致的快照。

**方法 B（沒有 `sqlite3`）**

```bash
docker compose stop urlet          # 正常關閉會 checkpoint WAL
docker run --rm -v urlet-data:/data -v "$PWD":/backup alpine \
  cp /data/urlet.db "/backup/urlet-$(date +%F).db"
docker compose start urlet
```

別忘了 `secret.key`：若你依賴自動產生的密鑰，備份它（或改用 `URLET_SECRET` 環境變數），
否則還原後所有 Session 與解鎖 Cookie 都會失效，既有邀請碼也無法再顯示（但仍可兌換）。
資料目錄備份同時包含資料庫與該密鑰，且邀請碼熵值低，請將整份備份視為敏感資料。

### 還原

```bash
docker compose down
docker run --rm -v urlet-data:/data -v "$PWD":/backup alpine \
  sh -c 'rm -f /data/urlet.db*; cp /backup/urlet-YYYY-MM-DD.db /data/urlet.db; chown 10001:10001 /data/urlet.db'
docker compose up -d
```

### 升級

```bash
git pull
docker compose up -d --build      # 啟動時自動套用遷移
```

使用預先建置的映像時，把 `URLET_IMAGE` 改成新版本並重建容器即可：

```bash
export URLET_IMAGE=ghcr.io/cxyfer/urlet:v1.2.0
docker compose -f docker-compose.yml -f docker-compose.image.yml up -d
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

- 一定要設定 `URLET_BASE_URL` 為使用者實際看到的網址，否則短網址會指向錯誤的網域。
- 只有在代理**保證覆寫** `X-Forwarded-For` / `X-Real-IP` 時才開啟 `URLET_TRUST_PROXY`，
  否則客戶端可以偽造 IP 繞過速率限制。程式優先採用 `X-Real-IP`（代理自行設定），
  其次取 `X-Forwarded-For` 最右側的位址，因此附加式代理也安全；
  隨附的 `deploy/Caddyfile` 已設定 `header_up X-Real-IP {remote_host}`。
- 需要傳遞 `Host`、`X-Forwarded-For`、`X-Forwarded-Proto`；Caddy 會自動處理。
- 邀請連結會在 `?invite=` 攜帶邀請碼；若反向代理的存取日誌記錄 query string，就會留下邀請碼。
  請限制或遮蔽該日誌，或改為只分享邀請碼。

## 9. 監控

- 健康檢查：`GET /health`（200 = 正常，503 = 資料庫異常）
- 容器 `HEALTHCHECK` 已內建，使用 `urlet healthcheck`（映像檔不需要 curl）
- 日誌輸出到 stdout；`URLET_LOG_FORMAT=json` 可搭配 Loki/CloudWatch 之類的收集器
- 每個請求都有 `x-request-id` 回應標頭，並以相同 id 寫入日誌
- 管理後台 **Admin → Overview** 可看使用者數、短網址數、點擊總量、資料庫大小

## 10. 疑難排解

| 症狀 | 原因與處理 |
|------|-----------|
| 啟動即失敗並顯示 migration checksum 不符 | `migrations/` 中已套用的檔案被修改；請還原該檔案或改用新的遷移檔 |
| 登入後馬上被登出 | `URLET_BASE_URL` 與實際網址不一致，或 `URLET_SECURE_COOKIES=true` 卻用 HTTP 存取 |
| 短網址指向 `http://localhost:8080` | 沒設定 `URLET_BASE_URL` |
| 容器啟動後 `/health` 回 503 | volume 權限問題：`/data` 必須可寫（容器以 uid 10001 執行） |
| 速率限制誤擋（NAT 後多個使用者） | 拉長限制或改用 API Token；限制是每 IP 計算 |
| 登入回 429 但密碼正確 | 帳號被暫時鎖定（15 分鐘內 30 次失敗）。等待視窗滑過，或 `sqlite3 /data/urlet.db "DELETE FROM login_attempts"` 立即解除 |
| 驗證元件完全沒出現 | 四個條件都要成立：provider 不是 `none`、兩把金鑰都非空、對應的 `ON_*` 為 `true` |
| 每次提交都回 `captcha_failed` | 密鑰不屬於目前設定的供應商、元件的網域清單沒有包含使用者實際連到的網域，或 API 用戶端忘了以 `captcha_token` 帶上驗證元件的權杖 |
| 使用者填完驗證碼仍無法註冊或登入 | token 解出的主機名稱不被 urlet 接受：請把它加入 `URLET_CAPTCHA_ALLOWED_HOSTNAMES`（`WARN` 日誌會列出回報的 hostname 與目前接受的候選清單） |
| 忘記管理員密碼後想用 Token 存取 | 重設密碼會撤銷該帳號所有 API Token（安全性設計），請重新建立 |
| 忘記管理員密碼 | 啟動時的 bootstrap 必須 `URLET_ADMIN_EMAIL` 與 `URLET_ADMIN_PASSWORD` **兩者皆非空**才會執行；對已存在的帳號只會提權，**不會**變更密碼（密碼僅在帳號不存在時使用）。救援方式：把 `URLET_ADMIN_EMAIL` 指向另一個你仍登得入的帳號（`URLET_ADMIN_PASSWORD` 隨便填一個非空值即可讓 bootstrap 執行），重啟後以該帳號登入；或直接用 `sqlite3` 覆寫密碼雜湊 |
| 短網址已過期想復原 | 到 `/app/urls/{id}` 清除或延長到期時間；資料未被刪除 |
| 刪除短網址後想重用同一個 slug | 刻意不允許（避免舊連結被劫持）；請改用新 slug |

## 11. 安全檢查清單

- [ ] `URLET_BASE_URL` 使用 `https://`
- [ ] `URLET_ADMIN_PASSWORD` 為強密碼，且登入後於 `/app/account` 更換
- [ ] 設定 `URLET_SECRET`（至少 32 個隨機字元）或確保自動產生的 `secret.key`（0600）已備份
- [ ] 對外只暴露 80/443（由 Caddy 或既有代理終結 TLS）
- [ ] 確認 `URLET_TRUST_PROXY` 的設定與代理行為一致
- [ ] 註冊模式設為 `invite` 或 `closed`（除非真的要開放註冊）
- [ ] 已規劃 `click_retention_days` 與備份排程
- [ ] 有固定的更新流程（本機重建或拉取新的 `URLET_IMAGE` 標籤），且每次升級前先備份
- [ ] 若實例可從公網連線，已為註冊與登入流程啟用防機器人驗證
