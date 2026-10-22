# urlet JSON API 參考

[English](../API.md) | **繁體中文**

- 基底路徑：`/api/v1`
- 認證方式（二選一）
  - **Session Cookie**：`POST /api/v1/auth/login` 取得的 `urlet_session`（瀏覽器表單流程同用）
  - **API Token**：`Authorization: Bearer ur_...`（在 `/app/account` 建立）
- 請求／回應皆為 `application/json; charset=utf-8`
- 時間一律為 ISO-8601 UTC（`2030-01-31T12:00:00.000Z`）；輸入接受
  `2030-01-31T12:00:00Z`、`2030-01-31 12:00`、`2030-01-31` 等形式
- JSON 回應一律為英文，不受使用者選擇的介面語言影響（見[介面語言](#介面語言)）

## 錯誤格式

```json
{
  "error": {
    "code": "validation_error",
    "message": "slug may only contain letters, digits, '-' and '_'",
    "details": { "field": "slug" }
  }
}
```

| HTTP | `code` | 說明 |
|------|--------|------|
| 400 | `bad_request` | 請求格式錯誤或沒有變更 |
| 400 | `captcha_failed` | 防機器人驗證失敗、逾期，或遭供應商拒絕（註冊與登入） |
| 401 | `unauthorized` | 未認證、Token 失效、密碼錯誤 |
| 403 | `forbidden` | 權限不足、CSRF 拒絕、註冊關閉、配額用盡 |
| 404 | `not_found` | 資源不存在（含不屬於你的資源） |
| 409 | `conflict` | 唯一性衝突（email、網域上的 slug、網域 hostname） |
| 410 | `gone` | 短網址已過期或達點擊上限 |
| 422 | `validation_error` | 欄位驗證失敗，`details.field` 指出欄位 |
| 429 | `rate_limited` | 速率限制，附 `Retry-After` 標頭 |
| 500 | `internal_error` | 伺服器錯誤（細節只寫入日誌） |

分頁格式：

```json
{ "items": [], "page": 1, "per_page": 20, "total": 0, "total_pages": 1 }
```

`per_page` 預設 20，上限 100。

---

## 認證

### `POST /api/v1/auth/register`

```json
{ "email": "user@example.com", "password": "至少 8 字元", "invite_code": "可選", "display_name": "可選", "captcha_token": "可選" }
```

`201` → `{ "user": { ... } }`，並在回應中設定 Session Cookie（註冊即登入）。

錯誤：`409` email 重複、`422` 驗證失敗、`403` 註冊關閉或邀請碼無效、
`400` `captcha_failed`（已啟用防機器人驗證且驗證未通過時）。

邀請碼可指定使用者群組。`invite` 模式下必須帶邀請碼；`open` 模式下為選填，但一旦提供就會驗證，
且其群組優先於預設群組。回應的 `user` 物件包含 `group_id`。

網頁表單也接受 `/app/register?invite=CODE` 的連結，會自動預填邀請碼；JSON 內容不變。

`captcha_token` 是網頁註冊表單上驗證元件的 token，只在註冊流程啟用防機器人驗證時檢查，
否則會忽略此欄位。詳見[防機器人驗證](#防機器人驗證)。

> `POST /api/v1/urls` **只有在使用 API Token（`Authorization: Bearer ur_...`）認證時**才不套用
> 防機器人驗證，因此自動化流程與 CI 不受影響。使用 **Session Cookie** 認證者屬於瀏覽器工作階段，
> 會受「建立短網址」開關約束，需在 `captcha_token` 帶入 widget token。

### `POST /api/v1/auth/login`

```json
{ "email": "user@example.com", "password": "...", "captcha_token": "可選" }
```

`200` → `{ "user": { ... } }` + `Set-Cookie`。
錯誤：`401` 帳密錯誤（訊息固定，不洩漏帳號是否存在）、`403` 帳號停權、
`400` `captcha_failed`（已為登入流程啟用防機器人驗證且驗證未通過時）、`429` 速率限制。

`captcha_token` 是網頁登入表單上驗證元件的 token，只在登入流程啟用防機器人驗證時檢查
（`captcha_on_login` / `URLET_CAPTCHA_ON_LOGIN`），否則會忽略此欄位。
詳見[防機器人驗證](#防機器人驗證)。

### `POST /api/v1/auth/logout`

`204`，撤銷當前 Session 並清除 Cookie（可匿名呼叫）。

### `GET /api/v1/auth/me`

`200` → 使用者、所屬群組（或 `null`）與生效額度：

```json
{
  "user": { "id": 7, "email": "ada@example.com", "role": "user", "group_id": 2, "...": "..." },
  "group": { "id": 2, "name": "Pro" },
  "limits": {
    "max_urls": 500,
    "max_api_tokens": 5,
    "allow_custom_slug": true,
    "allow_password": true,
    "allow_expiry": true,
    "allow_max_clicks": true,
    "allow_tags": true,
    "allow_api_tokens": true,
    "allow_domain_selection": true,
    "redirect_mode_policy": "choice",
    "default_domain": "links.example.com",
    "allowed_domains": ["links.example.com", "go.example.com"]
  }
}
```

未分組帳號的 `group` 為 `null`，改為套用全站設定。`limits` 是解析後的值：群組配額非 `NULL` 時
才覆蓋全站值，`0` 表示無上限。`default_domain` 是生效的群組預設網域（不存在或停用時回退全站
預設）；`allowed_domains` 是生效的可選清單。`allow_domain_selection` 為 `false` 時，建立連結
或移動連結只接受該預設網域。`redirect_mode_policy` 為 `direct`、`splash` 或 `choice`，
只限制**新**連結的轉址方式，不會改寫既有連結；未分組帳號為 `choice`。

### `PATCH /api/v1/auth/me`

```json
{ "display_name": "Ada" }
```

傳空字串或 `null` 可清除顯示名稱。`200` → `{ "user": { ... } }`

### `POST /api/v1/auth/password`

```json
{ "current_password": "...", "new_password": "..." }
```

`204`。成功後會撤銷**其他**所有 Session（當前這個保留）**以及該帳號的所有 API Token**。
錯誤：`401` 當前密碼錯誤、`422` 新密碼不符政策。

### `GET /api/v1/auth/sessions`

`200` → `{ "items": [ { "id", "user_agent", "created_at", "last_seen_at", "expires_at", "current", "revoked" } ] }`

Session 會滑動：每次活動（最多每分鐘寫入一次）都會把 `expires_at` 往後推，
並重新簽發 Cookie，因此持續使用不會過期。

### `DELETE /api/v1/auth/sessions/{id}`

`204`。只能撤銷自己的 Session，否則 `404`。

### `GET /api/v1/tokens` · `POST /api/v1/tokens` · `DELETE /api/v1/tokens/{id}`

`POST` 內容：`{ "name": "ci pipeline", "expires_at": "2030-01-01" }`

`201` → **明文 token 只會出現這一次**：

```json
{
  "token": "ur_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
  "api_token": { "id": 1, "name": "ci pipeline", "prefix": "ur_abc12345", "scopes": "...", "created_at": "...", "last_used_at": null, "expires_at": "2030-01-01T00:00:00.000Z", "revoked": false, "active": true }
}
```

`DELETE` → `204`。

群組關閉 API Token（`allow_api_tokens`）或生效的 `max_api_tokens` 用盡時，建立 Token 會回 `403`。
配額只計算可用 Token（未撤銷、未過期），因此撤銷後會立即釋出名額。

---

## 短網址

### `GET /api/v1/urls`

查詢參數：

| 參數 | 說明 |
|------|------|
| `q` | 搜尋 slug／標題／目標網址（`%`、`_` 視為字面值） |
| `tag` | 依標籤名稱篩選 |
| `status` | `active` 或 `disabled` |
| `sort` | `created_at`（預設）、`clicks`、`slug`、`updated`、`expires` |
| `page` / `per_page` | 分頁 |

`200` → 分頁的 `UrlDto` 陣列。

### `POST /api/v1/urls`

```json
{
  "target_url": "example.com/path",
  "domain": "go.example.com",
  "slug": "promo",
  "title": "Launch",
  "notes": "internal note",
  "tags": ["campaign", "docs"],
  "password": "linkpass",
  "expires_at": "2030-06-30 12:00",
  "max_clicks": 500,
  "redirect_type": 302,
  "redirect_mode": "splash",
  "splash_delay_seconds": 7,
  "splash_title": "Launch day",
  "splash_message": "Welcome aboard",
  "captcha_token": "0.abc..."
}
```

`201` → `UrlDto`。規則：

- `captcha_token` 只在「使用 **Session Cookie** 認證」且「建立短網址」的防機器人驗證已啟用時才會讀取；
  以 `Bearer` Token 呼叫時會被忽略。驗證失敗回 `400` 且 `error.code = "captcha_failed"`
- `target_url` 必填；只接受 `http`/`https`；未寫 scheme 的裸網域會自動補 `https://`
- `domain` 可省略（使用生效的群組預設網域）；指定時必須是 active 網域，否則回 `422`；還必須符合群組策略（`allow_domain_selection`、`restrict_domains` / `domain_ids`），否則回 `403`；slug 命名空間以網域為單位，同一個 slug 可以存在於兩個網域
- `slug` 可省略（自動產生 7 字元 base62）；僅允許 `[A-Za-z0-9_-]`，長度 1–64，且不得為保留字；若 slug **在選定網域上**已被使用，回 `409`
- `expires_at` 必須是未來時間；`max_clicks` ≥ 1
- `redirect_type` ∈ {301, 302, 307, 308}
- `redirect_mode` 為 `direct`（預設）或 `splash`。省略時採用群組政策的預設值（僅中轉頁政策下為
  `splash`，其餘為 `direct`）；群組封鎖的模式回 `403`
- `splash_delay_seconds` 介於 1–60（預設 5）。`splash_title`（最多 120 字元）與
  `splash_message`（最多 500 字元，可含換行）皆為選填；空白或省略時使用在地化的預設文案。
  即使連結目前是直接跳轉也會保存這些文案，切換模式時可直接沿用
- 超過 `max_urls_per_user` 配額 → `403`

### `GET /api/v1/urls/{id}`

`200` → `UrlDto`（`state` 為 `active` / `expired` / `limit_reached` / `disabled`）。

### `PATCH /api/v1/urls/{id}`

部分更新：**未出現的欄位維持不變，明確傳 `null` 表示清除**。

```json
{ "title": "新標題", "domain": "links.example.com", "password": null, "max_clicks": null, "tags": ["new"], "status": "disabled" }
```

`200` → 更新後的 `UrlDto`。空物件 → `400`。把連結搬到另一個 active 網域時必須符合群組策略，並重新檢查該網域的 slug，衝突回 `409`。維持目前網域（即使是已停用網域）一律允許，既有連結仍可編輯。

### `DELETE /api/v1/urls/{id}`

`204`，軟刪除。Slug 會保留不釋出（避免舊連結被劫持），統計資料保留。

### `GET /api/v1/urls/{id}/stats?days=30`

```json
{
  "url": { ... },
  "summary": { "total_clicks": 42, "human_clicks": 41, "unique_clicks": 3, "bot_clicks": 1, "first_click_at": "...", "last_click_at": "..." },
  "daily": [ { "day": "2026-02-01", "clicks": 5, "unique_clicks": 2, "bot_clicks": 0 } ],
  "referrers": [ { "label": "news.example.com", "count": 34 } ],
  "devices": [], "browsers": [], "operating_systems": []
}
```

`days` 範圍 1–365（預設 30），`daily` 會補齊沒有點擊的日期。

### `GET /api/v1/urls/{id}/clicks?page=1`

`200` → 點擊明細分頁（**不含** IP 或 visitor 雜湊）：

```json
{ "items": [ { "clicked_at": "...", "referrer_host": "news.example.com", "country_code": null, "device_type": "Desktop", "browser": "Chrome", "os": "Windows", "is_bot": false, "is_unique": true, "password_protected": false } ], "page": 1, "per_page": 20, "total": 42, "total_pages": 3 }
```

### `UrlDto`

```json
{
  "id": 1,
  "domain": "s.example.com",
  "slug": "promo",
  "short_url": "https://s.example.com/promo",
  "target_url": "https://example.com/landing",
  "title": "Launch",
  "notes": null,
  "tags": ["campaign"],
  "has_password": true,
  "expires_at": "2030-06-30T12:00:00.000Z",
  "expires_in": "in 52 months",
  "max_clicks": 500,
  "click_count": 42,
  "unique_click_count": 3,
  "redirect_type": 302,
  "redirect_mode": "splash",
  "splash_delay_seconds": 7,
  "splash_title": "Launch day",
  "splash_message": "Welcome aboard",
  "status": "active",
  "is_expired": false,
  "state": "active",
  "is_custom_slug": true,
  "last_clicked_at": "2026-02-11T09:00:00.000Z",
  "created_at": "...",
  "updated_at": "..."
}
```

**群組限制。** 生效的 `max_urls` 用盡時，建立短網址會回 `403`。群組功能開關會擋下建立時對應欄位
（`slug`、`password`、`expires_at`、`max_clicks`、`tags`）的提交，`PATCH` 時也不允許「新開啟」。
省略欄位代表維持原值，清除被封鎖的欄位則可接受；非空的連結密碼一律視為新值，在密碼被封鎖時會被拒絕。轉址方式政策會擋下新的
`redirect_mode`（或 `PATCH` 時切換到被封鎖的模式），但允許維持目前模式與清除中轉頁文案。

---

## 標籤

### `GET /api/v1/tags`

`200` → `{ "items": [ { "id": 1, "name": "campaign", "color": null, "url_count": 4 } ] }`

### `DELETE /api/v1/tags/{id}`

`204`，刪除標籤（僅解除與短網址的關聯，短網址本身不受影響）。

---

## 公開端點

### `GET /`

導覽首頁：介紹服務並提供登入／註冊入口。首頁不再 302 轉到 `/app`；
已登入的使用者會看到進入儀表板的按鈕。

當訪客被允許建立連結時（見下方的 `public_create_access` 設定），頁面也會帶上公開短網址工具。
`?shortened=<slug>&domain=<hostname>` 會渲染剛建立連結的短網址；該值以資料庫解析，因此結果框
永不逐字印出查詢字串的值，也永不揭露目的地。

### `POST /`

公開短網址工具表單（`application/x-www-form-urlencoded`），僅限 canonical 主機。它接受
`target_url`（必填），以及管理員已啟用的選填欄位：`slug`、`title`、`tags`（逗號分隔）、
`password`、`expires_at`、`max_clicks`、`redirect_mode`、`splash_delay_seconds`、
`splash_title`、`splash_message`、`domain`，以及供應商的權杖欄位
（`cf-turnstile-response` 或 `g-recaptcha-response`）。

送出成功為 `303` 轉往 `/?shortened=<slug>&domain=<hostname>`。失敗則為 `303` 轉往
`/?err=<code>`，其中 `<code>` 為 `denied`（低於存取層級，或填了被停用的欄位）、
`guest_allowlist`（匿名目標未命中白名單）、`rate_limited`、`captcha`、`quota`（訪客帳號的 `public_max_urls` 額度已滿）、
`conflict`、`slug`、`target`、`expiry`、`tags`、`password_rules`、`domain`、`mode_denied` 或
`invalid`。非數值的 `max_clicks` 或 `splash_delay_seconds` 會回報為 `invalid`，而不是默默丟棄。

表單套用 CSRF 同源規則，一般安全標頭維持不變。

### `POST /api/v1/public/urls`

與表單相同的操作，以 JSON 提供給指令稿使用。當 `public_create_access` 為 `guest` 時不需要任何
憑證；也可改帶 Session cookie 或 Bearer Token，此時連結由該帳號擁有。

匿名請求還必須命中已啟用的訪客目標白名單，下方範例已包含在內建規則中。登入 Session 與 Bearer Token 維持原有帳號權限。

```json
{
  "target_url": "https://github.com/cxyfer/urlet",
  "slug": "launch",
  "title": "Launch day",
  "captcha_token": "0.abc..."
}
```

`201`：

```json
{
  "domain": "s.example.com",
  "slug": "launch",
  "short_url": "https://s.example.com/launch",
  "target_url": "https://github.com/cxyfer/urlet",
  "created_at": "2026-10-22T12:00:00.000Z"
}
```

刻意不回傳連結 id：呼叫者沒有帳號可以管理該連結。

- `403` `forbidden` —— 呼叫者低於存取層級、送出了管理員停用的欄位，或匿名目標未命中已啟用的訪客白名單
- `400` `captcha_failed` —— 防機器人驗證失敗（fail closed）
- `429` `rate_limited` —— 每來源位址或全體訪客的額度用盡（`Retry-After`）
- `409` `conflict` —— 要求的 slug 已被使用
- `422` `validation_error` —— 目標網址或選項不合法

### `GET /health`

```json
{ "status": "ok", "version": "1.4.0-rc.2", "database": "ok", "uptime_seconds": 42 }
```

資料庫無法查詢時回 `503` 且 `status` 為 `degraded`。`/healthz` 是同一端點的別名。

### `GET /{slug}`

- 有效、無密碼且 `redirect_mode = direct` → `30x` + `Location`（301/308 為
  `private, max-age=300`，302/307 為 `private, no-store`）
- 有效、無密碼且 `redirect_mode = splash` → `200` 中轉頁（`no-store`）。頁面顯示自訂
  文案、目的地主機與倒數，並以 `<meta http-equiv="refresh">` 自動前往，同時提供「立即前往」
  按鈕。點擊是在中轉頁送出時記錄，不是訪客離開時
- 需要密碼 → `200` 密碼輸入頁（**不洩漏目標網址**）。解鎖成功後直接前往目的地：密碼頁是唯一
  的中間頁，因此受密碼保護的連結不會再顯示中轉頁
- 已過期或達點擊上限 → `410`
- 不存在／已刪除／已停用 → `404`

### `POST /{slug}/unlock`

接受 `application/json` 或 `application/x-www-form-urlencoded`：

```json
{ "password": "linkpass" }
```

- JSON 請求成功 → `200` `{ "ok": true, "redirect": "/slug" }` + 解鎖 Cookie
- 表單請求成功 → `303` 轉址到 `/{slug}` + 解鎖 Cookie
- 密碼錯誤 → JSON `401`；表單則重新渲染密碼頁（`401`）
- 超過速率限制 → `429`

### `GET /api/v1/public/urls/{slug}?domain=<hostname>`

公開中介資料，不含目標網址。省略 `domain` 時使用預設網域，讓多網域之前既有的呼叫對預設網域上的連結仍然有效。

```json
{ "domain": "go.example.com", "slug": "promo", "title": "Launch", "has_password": true, "redirect_mode": "splash", "is_expired": false, "state": "active" }
```

### `GET /api/v1/domains`

已登入使用者可列出建立短網址時可選的 active 網域；清單已依其所屬群組策略過濾，
`is_default` 標記生效的群組預設網域：

```json
[ { "id": 1, "hostname": "s.example.com", "is_default": true }, { "id": 2, "hostname": "go.example.com", "is_default": false } ]
```

---

## 介面語言

網頁介面提供三種語言：英文（`en`）、繁體中文（`zh-Hant`）與简体中文（`zh-Hans`）。
初次訪問時依瀏覽器的 `Accept-Language` 標頭自動選擇，無法辨識時使用英文；
選擇結果存在 cookie `urlet_lang`。

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/app/lang/{code}?next=<path>` | 切換介面語言、設定 `urlet_lang` cookie，接著回 `302` 轉往 `next`（未帶 `next` 或不是同站相對路徑時轉往 `/app`） |

`code` 為 `en`、`zh-Hant`、`zh-Hans` 之一；`next` 必須是同站的相對路徑。
無法辨識的 `code` 會退回英文（不會回錯誤）；未帶 cookie 的訪客則依 `Accept-Language`
協商，只要帶了 cookie 一律以 cookie 為準。

語言設定只影響渲染出的 HTML。**API 的 JSON 回應一律英文**，不受語言設定影響。

---

## 防機器人驗證

urlet 可以要求三個會產生資料的公開表單通過防機器人驗證：網頁註冊表單、網頁登入表單與
網頁建立短網址表單。JSON API 的 `POST /api/v1/auth/register` 與 `POST /api/v1/auth/login`
則以選填欄位 `captcha_token` 接收驗證元件的權杖。預設關閉，可透過環境變數設定，
也可在管理後台 **Admin → Settings** 設定；非空的環境變數會覆蓋後台設定。
變數與金鑰申請方式請見部署指南。

| 供應商 | `provider` 值 | 驗證元件 |
|--------|---------------|----------|
| 關閉（預設） | `none` | 無 |
| Cloudflare Turnstile | `turnstile` | Turnstile 元件（Managed 模式） |
| Google reCAPTCHA v2 | `recaptcha` | 「我不是機器人」核取方塊 |

四個條件都成立才會對某個表單啟用驗證：provider 不是 `none`、site key 非空、
secret key 非空，以及該表單對應的開關（`captcha_on_register`、`captcha_on_login`
或 `captcha_on_create`）為 `true`。

驗證失敗一律**拒絕**（fail closed）：token 缺少、格式錯誤、逾期或已使用過，
以及逾時或供應商回應非 2xx，都會拒絕該次提交。

供應商回報的 `hostname` 也會一併檢查：必須等於實例 base URL 的主機名稱，或
`captcha_allowed_hostnames` 中的其中一個（以逗號分隔，最多 10 項）。回應未帶 `hostname`
或該欄位為空字串時予以接受；不符時拒絕並記一筆 `WARN`。

| 套用位置 | 是否驗證 |
|----------|----------|
| 網頁註冊表單（`POST /app/register`） | 註冊流程啟用時套用 |
| 網頁登入表單（`POST /app/login`） | 登入流程啟用時套用 |
| 網頁建立短網址表單（`POST /app/urls`） | 建立流程啟用時套用 |
| `POST /api/v1/auth/register` | 註冊流程啟用時套用；以 `captcha_token` 傳入驗證元件的 token；失敗回 `400` 且 `error.code = "captcha_failed"` |
| `POST /api/v1/auth/login` | 登入流程啟用時套用；以 `captcha_token` 傳入驗證元件的 token；失敗回 `400` 且 `error.code = "captcha_failed"` |
| `POST /api/v1/urls` | **僅限 API Token 豁免** —— 帶 `Authorization: Bearer ur_...` 時不套用；使用 Session Cookie 者受「建立短網址」開關約束，可帶 optional `captcha_token` |

---

## 邀請碼（成員）

當群組開啟 `allow_invites` 時，成員可為**自己的群組**產生邀請碼。邀請碼會綁定該群組，
兌換者加入同一個群組。群組的日曆配額（`max_invites` / `invite_period`）同樣適用，且與
管理員發出的邀請碼共用。撤銷從未被兌換的邀請碼會退回額度；已被兌換的邀請碼則維持計入。

### `GET /api/v1/invites`

`GET /api/v1/invites?page=&per_page=` → `200`：自己產生的邀請碼（新到舊），
包含可顯示的邀請碼與註冊連結，以及生效配額。

```json
{
  "items": [
    { "id": 4, "prefix": "RW6M", "code": "RW6MF6C6JG",
      "link": "https://s.example.com/app/register?invite=RW6MF6C6JG",
      "note": "team", "group_id": 4, "max_uses": 2,
      "used_count": 1, "remaining": 1, "expires_at": null, "status": "active",
      "usable": true, "created_at": "2026-03-18T10:00:00.000Z" }
  ],
  "page": 1, "per_page": 20, "total": 1,
  "quota": { "group_id": 4, "group_name": "Invite Team", "allowed": true,
             "limit": 5, "period": "month", "used": 1, "remaining": 4 }
}
```

`code` 與 `link` 在 v1.3 之前建立、或 `URLET_SECRET` 更換後會是 `null`；`prefix` 一律存在。
已撤銷、次數用盡或到期的邀請碼只保留 `code` 供查閱，`link` 為 `null`。
`per_page` 預設 20、上限 100。`quota.allowed` 為 `false` 表示群組未開放成員發碼：
此時建立邀請碼會回 `403`，但關閉開關前已發出的邀請碼仍會列出並可撤銷。`limit` 為 `0`
（無上限）時 `remaining` 為 `null`。若帳號的群組連結遺失，會回退到預設群組。

成員發碼另有節流：每個帳號每小時最多 20 個（超過回 `429`）；群組配額仍是主要限制。
群組未設定有限配額時，除節流外即為無上限。

### `POST /api/v1/invites`

`{ "note"?: "team", "max_uses"?: 2 }` → `201`：

```json
{ "code": "RW6MF6C6JG", "link": "https://s.example.com/app/register?invite=RW6MF6C6JG",
  "invite": { "...": "..." }, "quota": { "...": "..." } }
```

邀請碼與連結也會由 `GET /api/v1/invites` 回傳，不會因關閉回應而遺失。頂層 `link` 與
`invite.link` 一致；若建立後的邀請碼已不可用，兩者都會是 `null`。成員發碼的
`max_uses` 必須介於 1–10。群組未開放或本週期配額用盡時回 `403`
（`error.code = "forbidden"`）；每小時爆量上限用盡時回 `429`。

### `DELETE /api/v1/invites/{id}`

`204`，撤銷自己的邀請碼。他人的邀請碼一律回 `404`；管理員可用
`DELETE /api/v1/admin/invites/{id}` 撤銷任何邀請碼。撤銷從未被兌換的邀請碼會退回配額；
已被兌換的邀請碼則維持計入。

---

## 管理（需 `role = admin`）

所有 `/api/v1/admin/**` 端點對非管理員回 `403 forbidden`。

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/admin/users?q=&role=&status=&group_id=&page=` | 使用者列表（含 `link_count`、`group_id`、`group_name`） |
| PATCH | `/admin/users/{id}` | `{ "role"?, "status"?, "display_name"?, "group_id"? }`；`group_id: null` 表示改為未分組 |
| POST | `/admin/users/{id}/password` | `{ "new_password": "..." }` → `204`，撤銷該帳號所有 Session |
| DELETE | `/admin/users/{id}` | `204`，軟刪除帳號與其所有短網址 |
| GET | `/admin/domains` | 所有網域，含連結數、狀態與來源 |
| POST | `/admin/domains` | `{ "hostname": "go.example.com" }` → `201`，建立為 `pending` |
| PATCH | `/admin/domains/{id}` | `{ "status"?: "active"\|"disabled", "is_default"?: true }` |
| DELETE | `/admin/domains/{id}` | `204`；僅限沒有連結、非預設的 UI 網域 |
| GET | `/admin/groups?page=&per_page=` | 使用者群組列表（含成員數，分頁） |
| POST | `/admin/groups` | `{ "name", "description"?, "max_urls"?, "max_api_tokens"?, "max_invites"?, "invite_period"?, "allow_invites"?, "allow_custom_slug"?, "allow_password"?, "allow_expiry"?, "allow_max_clicks"?, "allow_tags"?, "allow_api_tokens"?, "allow_domain_selection"?, "redirect_mode_policy"?, "default_domain_id"?, "restrict_domains"?, "domain_ids"?, "is_default"? }` → `201`；`default_domain_id` 必須指向 active 網域；`domain_ids` 必須指向存在的網域（停用的項目會保留但無法選取） |
| PUT | `/admin/groups/{id}` | `name` 為必填；其餘省略的欄位維持原值；`max_urls` / `max_api_tokens` / `max_invites` 設為 `null` 表示跟隨全站配額；`description: null` 會清除說明；`redirect_mode_policy` 為 `direct`、`splash` 或 `choice`，省略時維持原值；`default_domain_id: null` 表示全站預設；`domain_ids: null` 或 `[]` 會清空允許清單；`is_default: true` 會轉移唯一的預設標記 |
| DELETE | `/admin/groups/{id}?reassign_to={id}` | `204`，先移動成員再刪除；預設群組或目標不存在/不合法回 `409` |
| GET | `/admin/themes` | 主題目錄（key、名稱、說明、`enabled`、`default`、`appearance`、`dark_only`、`light_only`、`css_url`、`custom`）與 `switching`（`all` 或 `admin`）；包含管理員上傳主題 |
| PUT | `/admin/themes` | `{ "enabled"?: ["verdant", ...], "default"?: "verdant", "switching"?: "all"\|"admin" }`；省略欄位維持原值；至少一個啟用主題且預設必須啟用（否則 `422`）。`enabled` 與 `default` 可指定 `custom-…` 上傳主題 |

主題上傳是僅限管理員的瀏覽器 multipart 表單：`/app/admin/themes/custom`（建立）、
`/app/admin/themes/custom/{id}`（編輯／替換 CSS）與 `/app/admin/themes/custom/{id}/delete`。
JSON API 刻意不提供檔案上傳端點。上傳僅限 CSS、最大 256 KiB、UTF-8，且不可載入外部資源；
詳見 [`THEMES.md`](THEMES.md)。

`GroupDto` 另回傳 `default_domain_id`、`default_domain`（儲存值，即使網域已停用）、
`restrict_domains`、`allowed_domain_ids`、`allowed_domains`，以及邀請碼配額
（`max_invites`、`invite_period`）與目前用量（`invites_used`、`invites_remaining`，
無上限時為 `null`）。請求欄位是 `domain_ids`（`null` 或 `[]` 表示清空，省略則維持原值），回應使用
`allowed_*`。`GET /api/v1/auth/me` 回傳的是*生效*的 fallback 預設，若儲存的群組預設已停用，
可能與 `default_domain` 不同。
| GET | `/admin/urls?q=&status=&user_id=&page=` | 全站短網址 |
| PATCH | `/admin/urls/{id}` | `{ "status"?, "redirect_type"? }` |
| DELETE | `/admin/urls/{id}` | `204` |
| GET | `/admin/invites` | 邀請碼列表，含 `code`、`link`、`prefix`、`group_id` / `group_name`；舊版或密鑰更換後的列為 `null`；已撤銷、次數用盡或到期的邀請碼 `link` 也為 `null` |
| POST | `/admin/invites` | `{ "note"?, "max_uses"?, "expires_at"?, "group_id"? }` → `201` `{ "code", "link", "invite" }`；`link` 與 `invite.link` 一致，若建立後已不可用則為 `null`；`GET` 一律保留 `code`，`link` 只在可用時回傳；群組的週期配額用盡時回 `403` |
| DELETE | `/admin/invites/{id}` | `204`，撤銷；未被兌換的邀請碼會退回配額，已兌換的維持計入 |
| GET | `/admin/settings` | 目前設定，包含 `theme_switching`（`all` 或 `admin`） |
| PUT | `/admin/settings` | `{ "site_name": "...", "registration_mode": "invite", "theme_switching": "admin", ... }` |
| GET | `/admin/audit?action=&target_type=&actor=&page=` | 稽核日誌（`metadata` 已解析為 JSON） |
| GET | `/admin/stats` | 使用者／短網址／點擊總量、資料庫大小、版本、運行時間 |

**使用者群組。** 群組物件為
`{ id, name, description, max_urls, max_api_tokens, max_invites, invite_period, allow_*,
is_default, members, created_at, updated_at }`。`redirect_mode_policy` 為 `direct`、`splash` 或
`choice`（建立時預設 `choice`）。`allow_invites` 預設為 `false`（需明確開啟），
開啟後成員可自行發碼，並與管理員發出的邀請碼共用同一份配額。
配額可為 `null`（跟隨全站設定）或 `0`（無上限）。功能開關預設 `true`，只影響新的使用，
不會修改既有連結或 Token。管理員與一般使用者適用同一套規則。`PUT` 接受部分群組表示：省略欄位
即維持原值，配額設為 `null` 代表跟隨全站，`description: null` 會清除說明，`redirect_mode_policy` 為
`direct`、`splash` 或 `choice`，省略即維持原值，`is_default: true`
會轉移預設標記。`POST` 仍採建立預設值（省略開關視為允許、省略配額視為跟隨全站）。

**邀請碼配額。** 每個群組可限制每個日曆週期能「產生」的邀請碼數量。`max_invites` 可為
`null`（跟隨全站 `max_invites_per_period`）或 `0`（無上限）；`invite_period` 為 `day`、
`week`（週一起算）或 `month`，以 UTC 計算。超過上限會回 `403`，訊息會標明上限。計數保存在
發放流水帳中：撤銷從未被兌換的邀請碼會退回額度，已被兌換的邀請碼則維持計入；額度會在
週期交界自動重置。升級時也會釋放「已撤銷且從未兌換」的既有邀請碼，因此計數可能一次下降。
未指定 `group_id` 的邀請碼改為消耗全站配額。

**保護機制**：管理員不能變更自己的角色或狀態、不能刪除自己，且系統永遠保留至少一位
有效管理員（否則回 `409`）。預設群組不可刪除，請先以 `PUT` 轉移標記。

可編輯的設定鍵：`site_name`、`registration_mode`（`open`/`invite`/`closed`）、
`default_slug_length`（4–16）、`max_urls_per_user`（0 = 無限）、
`max_api_tokens_per_user`（0 = 無限，預設 20）、
`max_invites_per_period`（0 = 無限，預設 0）、`invite_period`（`day`/`week`/`month`，預設
`month`）、
`default_redirect_type`（301/302/307/308）、`analytics_enabled`、
`preview_image_policy`（`none`/`url`/`upload`/`both`，預設 `both`）、
`click_retention_days`（0 = 永久保留），以及防機器人驗證的
`captcha_provider`、`captcha_site_key`、`captcha_secret_key`、`captcha_on_register`、
`captcha_on_login`、`captcha_on_create`、`captcha_allowed_hostnames`。secret key 為唯寫：
永不回傳明文，留白送出表示維持已儲存的值。

公開短網址工具另新增 `public_create_access`（`guest`/`user`/`admin`，預設 `user`）、
`public_captcha`（預設 `true`）、`public_rate_limit_per_ip`（遊客每週期可建立數，預設 20，
`0` = 不限）、`public_rate_limit_per_ip_period`（預設 `hour`）、`public_rate_limit_global`
（全部遊客每週期合計可建立數，預設 500，`0` = 不限）、
`public_rate_limit_global_period`（預設 `hour`）、`public_max_urls`（保留的訪客帳號可擁有的連結總數，預設 1000，`0` = 無限制；與
`max_urls_per_user` 分開），以及欄位開關 `public_allow_slug`、`public_allow_title`（兩者預設 `true`），
再加上 `public_allow_tags`、`public_allow_password`、`public_allow_expiry`、
`public_allow_max_clicks`、`public_allow_redirect` 與 `public_allow_domain`（皆預設 `false`）。

遊客配額週期接受 `minute`、`hour`、`day`、`week`、`month`，使用服務進程的時區（`TZ`，未設定則採系統時區）。Docker Compose 預設 `TZ=UTC`，可例如設為 `TZ=Asia/Taipei`。後台顯示計數時區，且只有 `public_create_access=guest` 時才展開遊客速率、連結總額度、驗證與目標白名單設定。隱藏時保留原設定；無 JavaScript 時儲存存取層級後即可顯示欄位。首頁表單欄位開關因也套用於登入用戶而維持顯示。

訪客目標政策新增兩項字串設定：

| Key | 預設值 | 意義 |
|-----|--------|------|
| `guest_allowlist_enabled` | `"true"` | 僅限制匿名建立；`"false"` 明確關閉此政策 |
| `guest_allowlist` | 內建知名網站規則 | 每行一個 ASCII 主機名稱，可在開頭加上 `*.` 及區分大小寫的 ASCII 路徑，例如 `*.example.com/articles/` |

透過 `PUT /api/v1/admin/settings` 編輯任一項。主機名稱轉為小寫，移除空行與重複項目。萬用字元僅接受開頭的 `*.`；不接受協定、連接埠、帳密、其他萬用字元、查詢、片段、編碼規則路徑或點路徑。只有主機名稱時允許所有路徑；`/articles` 允許該路徑與子路徑，`/articles/` 則要求尾端斜線。查詢及片段不參與比對。限定路徑的規則拒絕目標中的編碼分隔符與巢狀編碼。僅允許 HTTP／HTTPS 及各自的標準連接埠。未加 `*.` 時主機名稱必須完全相符。`*.example.com` 依 DNS 標籤邊界涵蓋 `a.example.com` 與 `a.b.example.com`，但不包含 `example.com`、`evilexample.com` 或 `example.com.evil.test`；若要包含根網域，須另外列出 `example.com`。上限為 200 條規則與 16 KiB；無效更新回傳 `422`，整批設定維持原值。

啟用且清單為空時拒絕全部匿名目標；關閉政策會保留清單。更新立即生效且重啟保留，升級僅補齊缺少的 key。既有訪客連結仍可轉址。管理表單提供相同控制項，舊版表單省略新欄位時保留設定。訪客可在首頁查看目前允許的目標；表單遭拒時使用 `?err=guest_allowlist`。

內建清單包含 24 個常見服務、43 條主機規則。這些項目允許所列主機的所有路徑；管理員仍可在自訂規則中加上路徑限制。需要時分別列出根網域與子網域，預設範圍如下：

| 類別 | 服務與允許的主機 |
|------|------------------|
| 知識與開發 | Wikipedia（`wikipedia.org`、`*.wikipedia.org`）、GitHub（`github.com`）、GitLab（`gitlab.com`）、Stack Overflow（`stackoverflow.com`、`*.stackoverflow.com`）、Stack Exchange（`stackexchange.com`、`*.stackexchange.com`）、MDN（`developer.mozilla.org`）、Python 文件（`docs.python.org`）、Rust 文件（`doc.rust-lang.org`） |
| 台灣討論區 | PTT（`ptt.cc`、`www.ptt.cc`）、Dcard（`dcard.tw`、`www.dcard.tw`）、巴哈姆特（`gamer.com.tw`、`*.gamer.com.tw`） |
| 影音與音樂 | YouTube（`*.youtube.com`、`youtube.com`）、Bilibili（`bilibili.com`、`*.bilibili.com`）、Twitch（`twitch.tv`、`*.twitch.tv`）、Spotify（`spotify.com`、`*.spotify.com`） |
| 社群 | Reddit（`reddit.com`、`*.reddit.com`）、Instagram（`instagram.com`、`www.instagram.com`）、Facebook（`facebook.com`、`*.facebook.com`）、X（`x.com`、`www.x.com`） |
| 新聞 | 中央社（`cna.com.tw`、`www.cna.com.tw`）、BBC（`bbc.com`、`www.bbc.com`、`bbc.co.uk`、`www.bbc.co.uk`）、Reuters（`reuters.com`、`www.reuters.com`） |
| 產品支援 | Apple（`support.apple.com`）、Microsoft（`support.microsoft.com`） |

未內建 `*.github.io`、`*.gitlab.io`、`sites.google.com` 等公開網站代管網域。GitHub 與 GitLab 官方文件說明使用者可在 Pages 網域發布自己的網站，因此允許主站不代表連同這些獨立的代管網域一起允許，參見官方 [GitHub Pages](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages) 與 [GitLab Pages](https://docs.gitlab.com/user/project/pages/) 文件。這是初始政策，不是安全認證：知名網站仍可能包含使用者內容或轉址。沒有可信度 API、目標內容抓取或轉址鏈檢查。

---

### 追蹤參數清理

管理員可在 **管理 → 設定** 啟用，或使用 `PUT /api/v1/admin/settings`：

```json
{
  "url_cleaning_enabled": "true",
  "url_cleaning_rules": "example.com utm_* ref\nexample.org campaign_id"
}
```

`GET /api/v1/admin/settings` 會回傳這兩個設定，預設為 `"false"` 及 `""`；與其他設定相同，值皆為字串。僅管理員可讀取或修改。非法設定回傳 `422`，整個批次都不會改變資料庫或設定快取。

每行一條 `網域 參數 [參數 ...]`，比對該網域及其子網域，網域及解碼後的參數名稱忽略 ASCII 大小寫。非空參數名稱尾端的 `*` 表示前綴比對。空白行及以 `#` 開頭的整行註解會略過。上限為 8192 bytes、100 條規則，每條 32 個參數、每個參數 64 bytes（含 `*`）。網域須為 ASCII hostname，至少兩個非空分段，總長最多 253 bytes、每段最多 63 bytes；分段限英文字母、數字或連字號，不得以連字號開頭或結尾。拒絕 scheme、port、路徑、帳密及萬用網域。參數限英文字母、數字、`_`、`-` 或 `.`，尾端可加一個 `*`；拒絕單獨 `*` 及正規表示式。

內建規則涵蓋 Facebook（`facebook.com`）、Instagram、Threads（`threads.net`、`threads.com`）、X/Twitter、YouTube（`youtube.com`、`youtu.be`）、TikTok、Bilibili 及其子網域；移除 `utm_*` 與明確的平台追蹤參數，例如 `fbclid`、`igsh`、YouTube 的 `si`、Bilibili 的 `spm_id_from`，保留影片 ID、播放清單及播放時間。管理設定頁可查看完整清單。自訂規則與內建規則合併，留空則只使用內建規則。未知網域除非符合自訂規則，否則保持原樣。自訂規則若移除必要參數，仍可能改變目的內容。

清理**僅套用於建立連結**，包含 `POST /api/v1/urls`、`POST /api/v1/public/urls`、後台及公開表單。例如 `https://youtu.be/ABC?si=tracking&t=90` 儲存及回傳為 `https://youtu.be/ABC?t=90`。原始目標須先通過驗證；保留的 query 元件維持原始編碼、值與順序，fragment 保持完整。不展開分享短鏈，也不讀取目標內容。既有連結、目標網址編輯及重新導向維持原樣。停用功能仍保留自訂規則，供日後使用。

---

## 稽核動作代碼

| 動作 | 說明 |
|------|------|
| `auth.register` / `auth.login` / `auth.login_failed` / `auth.logout` | 認證事件 |
| `auth.password_changed` / `auth.session_revoked` / `user.profile_updated` | 帳號異動 |
| `url.created` / `url.updated` / `url.deleted` | 短網址異動 |
| `token.created` / `token.revoked` | API Token |
| `invite.created` / `invite.revoked` / `invite.redeemed` | 邀請碼 |
| `admin.user_updated` / `admin.user_deleted` / `admin.password_reset` | 管理操作 |
| `admin.group_created` / `admin.group_updated` / `admin.group_deleted` | 群組異動 |
| `admin.url_updated` / `admin.settings_updated` / `admin.themes_updated` / `admin.theme_created` / `admin.theme_updated` / `admin.theme_deleted` | 管理操作 |

## 速率限制

| 對象 | 限制 |
|------|------|
| 登入 | 每 (IP, email) 5 分鐘 10 次，**以及**每 IP 5 分鐘 40 次（跨帳號） |
| 註冊 | 每 IP 每小時 5 次 |
| 解鎖短網址 | 每 (slug, IP) 5 分鐘 10 次 |
| 建立短網址 | 每使用者每小時 120 次 |
| 公開短網址工具 | 只限未登入遊客：`public_rate_limit_per_ip` 依 `public_rate_limit_per_ip_period` 計數，`public_rate_limit_global` 依 `public_rate_limit_global_period` 計數（預設每小時 20／500 次）。週期為伺服器時區的自然分鐘、時、日、週、月，週一為週首，月份按曆月。`0` 停用任一配額。單一來源配額在驗證前扣抵，共用配額只在成功後扣抵。SQLite 用量跨重啟保留；`Retry-After` 為距離下個自然邊界的秒數。登入用戶維持分組額度與原有帳號防濫用機制，不消耗遊客配額 |
| 修改密碼 | 每使用者每小時 10 次 |

遊客持久化配額表中，所有來源、共用配額與週期合計最多 50,000 列。滿額時新列回 `429`，既有計數仍保留用量與限制。扣抵前會立即回收已到期週期，維護時也會清理；不會為了新來源淘汰未到期計數。

超過限制回 `429` 並附 `Retry-After`（秒）。記憶體計數器重啟即重置，
但**帳號鎖定**會持久化：同一個 email 於 15 分鐘內累積 30 次失敗後，
即使密碼正確也會回 `429`（`Retry-After: 900`），直到視窗滑過為止。

登入的 IP 來源：`URLET_TRUST_PROXY=true` 時優先採用 `X-Real-IP`，
其次取 `X-Forwarded-For` 最右側的位址。


## SNS 預覽設定

管理員可在 `/app/admin/settings` 或以 `PUT /api/v1/admin/settings` 設定 `preview_image_policy`，例如 `{"preview_image_policy":"none"}`。值為 `none`（不允許圖片）、`url`（僅 HTTPS 網址）、`upload`（僅上傳）、`both`（預設皆可）。非法值回 422，整批不寫入。政策同時套用管理員與一般成員；API 設定禁止的來源回 403，瀏覽器表單回保留文字的 422 重試頁。保留／移除圖片與編輯文字仍可使用。不允許的既有圖片仍儲存，但不輸出 metadata 圖片，回應的 `image_url` 為 null；`image_uploaded` 仍表示已儲存的來源。禁止上傳時，上傳圖片網址回 404 與 no-store；重新允許後恢復。第三方已複製或快取的公開圖片無法撤回。啟動時只補入缺少的設定，舊設定表單缺少此欄位時保留原政策。


社群平台抓取網頁的 Open Graph（`og:title`、`og:description`、`og:image`）及 Twitter Card 標籤來產生預覽。未自訂時，urlet 沿用轉址行為，平台可能從目標網站取得 metadata；自訂後，已知社群爬蟲取得的是擁有者儲存的預覽欄位。Footer、版型與更新時間由平台決定，這些標籤沒有任意 footer 欄位。

端點要求短網址擁有者的 session 或 API token；其他人的連結回傳 404。Cookie 請求沿用既有 CSRF 規則。先建立短網址，再設定選填的公開預覽；公開短網址工具不接受圖片上傳或這些額外欄位。

| 方法 | 路徑 | 行為 |
|------|------|------|
| GET | `/api/v1/urls/{id}/preview` | 回傳 `{title, description, image_url, image_uploaded, enabled}`；未設定時各欄位為 null，旗標為 false |
| PATCH | `/api/v1/urls/{id}/preview` | JSON `{title?, description?, image_url?}`；省略則保留，null 或空白則清除。設定／清除 image_url 會替換／移除上傳圖片；回傳格式同 GET |
| POST | `/api/v1/urls/{id}/preview/image` | Multipart 欄位 `image`；替換圖片並保留文字，回傳格式同 GET |
| DELETE | `/api/v1/urls/{id}/preview` | 清除所有自訂預覽與圖片資料，回傳 204 |

```bash
curl -X PATCH "$BASE/api/v1/urls/42/preview" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"title":"閱讀內容","description":"自訂摘要","image_url":"https://example.com/card.png"}'
curl "$BASE/api/v1/urls/42/preview/image" \
  -H "Authorization: Bearer $TOKEN" -F 'image=@card.png'
```

標題上限 200 字，內容上限 1000 字。圖片網址須為完整 HTTPS，上限 2048 字，不含帳號密碼或空白。上傳支援 PNG、JPEG、GIF、WebP，上限 2 MiB、4096 × 4096 像素；依驗證後的檔案內容決定 MIME，不相信檔名。圖片存於 SQLite，以不透明的公開 `/assets/link-previews/{key}` 網址提供；替換／移除後舊網址失效。欄位無效回傳 422；格式錯誤或過大的 multipart 回傳 4xx，既有資料不變。上傳套用每帳號速率限制。

預覽文字與圖片即使在密碼保護連結也會公開。預覽標題留空時使用站台名稱；管理用標題、備註與目標網址不作預設內容。伺服器不抓取外部 metadata 或圖片。已知社群爬蟲（含 Discord、X／Twitter、Meta、Telegram、Slack、LinkedIn、LINE 標記）會取得 OG／Twitter HTML；未知 agent 保留一般轉址行為。Metadata 與上傳圖片都遵循連結生命週期；自訂預覽抓取不計點擊。第三方快取與版型由平台決定，修改後不保證立即更新卡片。
