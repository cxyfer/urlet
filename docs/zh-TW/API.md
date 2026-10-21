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
    "default_domain": "links.example.com",
    "allowed_domains": ["links.example.com", "go.example.com"]
  }
}
```

未分組帳號的 `group` 為 `null`，改為套用全站設定。`limits` 是解析後的值：群組配額非 `NULL` 時
才覆蓋全站值，`0` 表示無上限。`default_domain` 是生效的群組預設網域（不存在或停用時回退全站
預設）；`allowed_domains` 是生效的可選清單。`allow_domain_selection` 為 `false` 時，建立連結
或移動連結只接受該預設網域。

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
省略欄位代表維持原值，清除被封鎖的欄位則可接受；非空的連結密碼一律視為新值，在密碼被封鎖時會被拒絕。

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

### `GET /health`

```json
{ "status": "ok", "version": "1.3.0-rc.1", "database": "ok", "uptime_seconds": 42 }
```

資料庫無法查詢時回 `503` 且 `status` 為 `degraded`。`/healthz` 是同一端點的別名。

### `GET /{slug}`

- 有效且無密碼 → `30x` + `Location`（301/308 為 `private, max-age=300`，302/307 為 `private, no-store`）
- 需要密碼 → `200` 密碼輸入頁（**不洩漏目標網址**）
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
{ "domain": "go.example.com", "slug": "promo", "title": "Launch", "has_password": true, "is_expired": false, "state": "active" }
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
管理員發出的邀請碼共用；撤銷邀請碼不會退回額度。

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

邀請碼與連結也會由 `GET /api/v1/invites` 回傳，不會因關閉回應而遺失。成員發碼的
`max_uses` 必須介於 1–10。群組未開放或本週期配額用盡時回 `403`
（`error.code = "forbidden"`）；每小時爆量上限用盡時回 `429`。

### `DELETE /api/v1/invites/{id}`

`204`，撤銷自己的邀請碼。他人的邀請碼一律回 `404`；管理員可用
`DELETE /api/v1/admin/invites/{id}` 撤銷任何邀請碼。

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
| POST | `/admin/groups` | `{ "name", "description"?, "max_urls"?, "max_api_tokens"?, "max_invites"?, "invite_period"?, "allow_invites"?, "allow_custom_slug"?, "allow_password"?, "allow_expiry"?, "allow_max_clicks"?, "allow_tags"?, "allow_api_tokens"?, "allow_domain_selection"?, "default_domain_id"?, "restrict_domains"?, "domain_ids"?, "is_default"? }` → `201`；`default_domain_id` 必須指向 active 網域；`domain_ids` 必須指向存在的網域（停用的項目會保留但無法選取） |
| PUT | `/admin/groups/{id}` | `name` 為必填；其餘省略的欄位維持原值；`max_urls` / `max_api_tokens` / `max_invites` 設為 `null` 表示跟隨全站配額；`description: null` 會清除說明；`default_domain_id: null` 表示全站預設；`domain_ids: null` 或 `[]` 會清空允許清單；`is_default: true` 會轉移唯一的預設標記 |
| DELETE | `/admin/groups/{id}?reassign_to={id}` | `204`，先移動成員再刪除；預設群組或目標不存在/不合法回 `409` |

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
| POST | `/admin/invites` | `{ "note"?, "max_uses"?, "expires_at"?, "group_id"? }` → `201` `{ "code", "link", "invite" }`；同一組 code 與 link 之後仍可由 `GET` 取得；群組的週期配額用盡時回 `403` |
| DELETE | `/admin/invites/{id}` | `204`，撤銷 |
| GET | `/admin/settings` | 目前設定 |
| PUT | `/admin/settings` | `{ "site_name": "...", "registration_mode": "invite", ... }` |
| GET | `/admin/audit?action=&target_type=&actor=&page=` | 稽核日誌（`metadata` 已解析為 JSON） |
| GET | `/admin/stats` | 使用者／短網址／點擊總量、資料庫大小、版本、運行時間 |

**使用者群組。** 群組物件為
`{ id, name, description, max_urls, max_api_tokens, max_invites, invite_period, allow_*,
is_default, members, created_at, updated_at }`。`allow_invites` 預設為 `false`（需明確開啟），
開啟後成員可自行發碼，並與管理員發出的邀請碼共用同一份配額。
配額可為 `null`（跟隨全站設定）或 `0`（無上限）。功能開關預設 `true`，只影響新的使用，
不會修改既有連結或 Token。管理員與一般使用者適用同一套規則。`PUT` 接受部分群組表示：省略欄位
即維持原值，配額設為 `null` 代表跟隨全站，`description: null` 會清除說明，`is_default: true`
會轉移預設標記。`POST` 仍採建立預設值（省略開關視為允許、省略配額視為跟隨全站）。

**邀請碼配額。** 每個群組可限制每個日曆週期能「產生」的邀請碼數量。`max_invites` 可為
`null`（跟隨全站 `max_invites_per_period`）或 `0`（無上限）；`invite_period` 為 `day`、
`week`（週一起算）或 `month`，以 UTC 計算。超過上限會回 `403`，訊息會標明上限。計數保存在
發放流水帳中，因此**撤銷邀請碼不會退回額度**，額度會在週期交界自動重置。未指定 `group_id`
的邀請碼改為消耗全站配額。

**保護機制**：管理員不能變更自己的角色或狀態、不能刪除自己，且系統永遠保留至少一位
有效管理員（否則回 `409`）。預設群組不可刪除，請先以 `PUT` 轉移標記。

可編輯的設定鍵：`site_name`、`registration_mode`（`open`/`invite`/`closed`）、
`default_slug_length`（4–16）、`max_urls_per_user`（0 = 無限）、
`max_api_tokens_per_user`（0 = 無限，預設 20）、
`max_invites_per_period`（0 = 無限，預設 0）、`invite_period`（`day`/`week`/`month`，預設
`month`）、
`default_redirect_type`（301/302/307/308）、`analytics_enabled`、
`click_retention_days`（0 = 永久保留），以及防機器人驗證的
`captcha_provider`、`captcha_site_key`、`captcha_secret_key`、`captcha_on_register`、
`captcha_on_login`、`captcha_on_create`、`captcha_allowed_hostnames`。secret key 為唯寫：
永不回傳明文，留白送出表示維持已儲存的值。

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
| `admin.url_updated` / `admin.settings_updated` | 管理操作 |

## 速率限制

| 對象 | 限制 |
|------|------|
| 登入 | 每 (IP, email) 5 分鐘 10 次，**以及**每 IP 5 分鐘 40 次（跨帳號） |
| 註冊 | 每 IP 每小時 5 次 |
| 解鎖短網址 | 每 (slug, IP) 5 分鐘 10 次 |
| 建立短網址 | 每使用者每小時 120 次 |
| 修改密碼 | 每使用者每小時 10 次 |

超過限制回 `429` 並附 `Retry-After`（秒）。記憶體計數器重啟即重置，
但**帳號鎖定**會持久化：同一個 email 於 15 分鐘內累積 30 次失敗後，
即使密碼正確也會回 `429`（`Retry-After: 900`），直到視窗滑過為止。

登入的 IP 來源：`URLET_TRUST_PROXY=true` 時優先採用 `X-Real-IP`，
其次取 `X-Forwarded-For` 最右側的位址。
