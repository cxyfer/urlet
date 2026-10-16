# uwurls JSON API 參考

[English](../API.md) | **繁體中文**

- 基底路徑：`/api/v1`
- 認證方式（二選一）
  - **Session Cookie**：`POST /api/v1/auth/login` 取得的 `uwurls_session`（瀏覽器表單流程同用）
  - **API Token**：`Authorization: Bearer uw_...`（在 `/app/account` 建立）
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
| 400 | `captcha_failed` | 防機器人驗證失敗、逾期，或遭供應商拒絕（僅註冊） |
| 401 | `unauthorized` | 未認證、Token 失效、密碼錯誤 |
| 403 | `forbidden` | 權限不足、CSRF 拒絕、註冊關閉、配額用盡 |
| 404 | `not_found` | 資源不存在（含不屬於你的資源） |
| 409 | `conflict` | 唯一性衝突（email、slug） |
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

`captcha_token` 是網頁註冊表單上驗證元件的 token，只在註冊流程啟用防機器人驗證時檢查，
否則會忽略此欄位。詳見[防機器人驗證](#防機器人驗證)。

> `POST /api/v1/urls` 使用 Session Cookie 或 API Token 認證，**不套用**防機器人驗證，
> 自動化流程與 CI 不受影響。

### `POST /api/v1/auth/login`

```json
{ "email": "user@example.com", "password": "..." }
```

`200` → `{ "user": { ... } }` + `Set-Cookie`。
錯誤：`401` 帳密錯誤（訊息固定，不洩漏帳號是否存在）、`403` 帳號停權、`429` 速率限制。

### `POST /api/v1/auth/logout`

`204`，撤銷當前 Session 並清除 Cookie（可匿名呼叫）。

### `GET /api/v1/auth/me`

`200` → `{ "user": { ... } }`

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
  "token": "uw_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
  "api_token": { "id": 1, "name": "ci pipeline", "prefix": "uw_abc12345", "scopes": "...", "created_at": "...", "last_used_at": null, "expires_at": "2030-01-01T00:00:00.000Z", "revoked": false, "active": true }
}
```

`DELETE` → `204`。

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
  "slug": "promo",
  "title": "Launch",
  "notes": "internal note",
  "tags": ["campaign", "docs"],
  "password": "linkpass",
  "expires_at": "2030-06-30 12:00",
  "max_clicks": 500,
  "redirect_type": 302
}
```

`201` → `UrlDto`。規則：

- `target_url` 必填；只接受 `http`/`https`；未寫 scheme 的裸網域會自動補 `https://`
- `slug` 可省略（自動產生 7 字元 base62）；僅允許 `[A-Za-z0-9_-]`，長度 1–64，且不得為保留字
- `expires_at` 必須是未來時間；`max_clicks` ≥ 1
- `redirect_type` ∈ {301, 302, 307, 308}
- 超過 `max_urls_per_user` 配額 → `403`

### `GET /api/v1/urls/{id}`

`200` → `UrlDto`（`state` 為 `active` / `expired` / `limit_reached` / `disabled`）。

### `PATCH /api/v1/urls/{id}`

部分更新：**未出現的欄位維持不變，明確傳 `null` 表示清除**。

```json
{ "title": "新標題", "password": null, "max_clicks": null, "tags": ["new"], "status": "disabled" }
```

`200` → 更新後的 `UrlDto`。空物件 → `400`。

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
{ "status": "ok", "version": "1.0.0", "database": "ok", "uptime_seconds": 42 }
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

### `GET /api/v1/public/urls/{slug}`

公開中介資料，不含目標網址：

```json
{ "slug": "promo", "title": "Launch", "has_password": true, "is_expired": false, "state": "active" }
```

---

## 介面語言

網頁介面提供三種語言：英文（`en`）、繁體中文（`zh-Hant`）與简体中文（`zh-Hans`）。
初次訪問時依瀏覽器的 `Accept-Language` 標頭自動選擇，無法辨識時使用英文；
選擇結果存在 cookie `uwurls_lang`。

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/app/lang/{code}?next=<path>` | 切換介面語言，之後轉回 `next`（未帶 `next` 時回到 `/`） |

`code` 為 `en`、`zh-Hant`、`zh-Hans` 之一；`next` 必須是同站的相對路徑。
無法辨識的 `code` 會退回英文。

語言設定只影響渲染出的 HTML。**API 的 JSON 回應一律英文**，不受語言設定影響。

---

## 防機器人驗證

uwurls 可以要求兩個會產生資料的公開表單通過防機器人驗證：網頁註冊表單與網頁建立短網址表單。
預設關閉，可透過環境變數設定，也可在管理後台 **Admin → Settings** 設定；
非空的環境變數會覆蓋後台設定。變數與金鑰申請方式請見部署指南。

| 供應商 | `provider` 值 | 驗證元件 |
|--------|---------------|----------|
| 關閉（預設） | `none` | 無 |
| Cloudflare Turnstile | `turnstile` | Turnstile 元件（Managed 模式） |
| Google reCAPTCHA v2 | `recaptcha` | 「我不是機器人」核取方塊 |

四個條件都成立才會對某個表單啟用驗證：provider 不是 `none`、site key 非空、
secret key 非空，以及該表單對應的開關為 `true`。

驗證失敗一律**拒絕**（fail closed）：token 缺少、格式錯誤、逾期或已使用過，
以及逾時或供應商回應非 2xx，都會拒絕該次提交。

| 套用位置 | 是否驗證 |
|----------|----------|
| 網頁註冊表單（`POST /app/register`） | 註冊流程啟用時套用 |
| 網頁建立短網址表單（`POST /app/urls`） | 建立流程啟用時套用 |
| `POST /api/v1/auth/register` | 註冊流程啟用時套用；以 `captcha_token` 傳入驗證元件的 token；失敗回 `400` 且 `error.code = "captcha_failed"` |
| `POST /api/v1/urls` | **不套用**，以 API Token 或 Session Cookie 認證 |

---

## 管理（需 `role = admin`）

所有 `/api/v1/admin/**` 端點對非管理員回 `403 forbidden`。

| 方法 | 路徑 | 說明 |
|------|------|------|
| GET | `/admin/users?q=&role=&status=&page=` | 使用者列表（含 `link_count`） |
| PATCH | `/admin/users/{id}` | `{ "role"?, "status"?, "display_name"? }` |
| POST | `/admin/users/{id}/password` | `{ "new_password": "..." }` → `204`，撤銷該帳號所有 Session |
| DELETE | `/admin/users/{id}` | `204`，軟刪除帳號與其所有短網址 |
| GET | `/admin/urls?q=&status=&user_id=&page=` | 全站短網址 |
| PATCH | `/admin/urls/{id}` | `{ "status"?, "redirect_type"? }` |
| DELETE | `/admin/urls/{id}` | `204` |
| GET | `/admin/invites` | 邀請碼列表（僅顯示前綴） |
| POST | `/admin/invites` | `{ "note"?, "max_uses"?, "expires_at"? }` → `201`，**明文邀請碼只回一次** |
| DELETE | `/admin/invites/{id}` | `204`，撤銷 |
| GET | `/admin/settings` | 目前設定 |
| PUT | `/admin/settings` | `{ "site_name": "...", "registration_mode": "invite", ... }` |
| GET | `/admin/audit?action=&target_type=&actor=&page=` | 稽核日誌（`metadata` 已解析為 JSON） |
| GET | `/admin/stats` | 使用者／短網址／點擊總量、資料庫大小、版本、運行時間 |

**保護機制**：管理員不能變更自己的角色或狀態、不能刪除自己，且系統永遠保留至少一位
有效管理員（否則回 `409`）。

可編輯的設定鍵：`site_name`、`registration_mode`（`open`/`invite`/`closed`）、
`default_slug_length`（4–16）、`max_urls_per_user`（0 = 無限）、
`default_redirect_type`（301/302/307/308）、`analytics_enabled`、
`click_retention_days`（0 = 永久保留）。

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

登入的 IP 來源：`UWURLS_TRUST_PROXY=true` 時優先採用 `X-Real-IP`，
其次取 `X-Forwarded-For` 最右側的位址。
