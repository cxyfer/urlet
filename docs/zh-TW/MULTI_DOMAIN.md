# 多網域支援設計

[English](../MULTI_DOMAIN.md) | **繁體中文**

狀態：已實作（v1.3.0 起釋出）
最後更新：2026-10-28

## 1. 摘要

urlet 以一個 **canonical host** 提供完整應用，並可加上任意數量的
**額外網域**。Canonical host 保留 `/`、`/app/*`、`/api/v1/*`；額外網域只做
轉址：回應 `GET /{slug}`、`POST /{slug}/unlock`，根路徑顯示主題感知的短網址
首頁，其餘路徑導回 canonical 或拒絕。

每個網域有自己的 slug 命名空間：`(domain_id, slug)` 唯一。同一個 slug 可以
同時存在於 `a.example.com` 與 `b.example.com`，各自指向不同目標。

## 2. 產品決策

| # | 決策 | 選擇 |
|---|------|------|
| 1 | 額外網域範圍 | 只做轉址。`/app/*` 的 GET/HEAD → 308 回 canonical；其他方法 → 404。`/api/v1/*` → 404 JSON。 |
| 2 | 額外網域根路徑 | 主題感知的短網址首頁（`noindex`），七個主題皆適用，並提供站台工具（主題選擇器、語言切換）；不是 canonical 的行銷首頁，也不是 404。 |
| 3 | 預設網域 | 管理員可在 active 網域之間改動。Canonical host 永遠存在且永遠 active，但不一定是預設。 |
| 4 | 新網域 | 建立後為 `pending`；管理員確認 DNS/TLS 後再啟用。只有 `active` 網域可被選用。 |
| 5 | Scheme | 全站單一 scheme，繼承 `URLET_BASE_URL`；不新增 per-domain scheme 欄位。 |
| 6 | 公開 metadata API | 新增選用 `?domain=<hostname>`；省略時查預設網域。 |
| 7 | 環境變數 seed | `URLET_DOMAINS` 只補缺少的網域，不改既有網域的狀態、預設或驗證。 |

其餘細節的實作預設：

- 由 `URLET_DOMAINS` seed 的網域直接插入為 `active`、已驗證，因為那是營運者
  在部署設定中的明確宣告。
- 短網址首頁會依七個主題呈現，顯示站名、一句說明、回 canonical 的連結，以及
  站台工具（主題選擇器、語言切換）；沒有登入、註冊或短碼輸入框。
- 目前的預設網域不能停用或刪除，必須先把另一個 active 網域設為預設。
- `domains.source` 區分 `system`、`env`、`ui`。System/canonical 網域不可停用、
  也不可刪除；env 網域可以停用，但不能從 UI 刪除。

## 3. Host 角色與路由

| Host | `/` | `/{slug}` | `/app/*` | `/api/v1/*` | `/health` |
|------|-----|-----------|----------|-------------|-----------|
| canonical | 既有行銷首頁 | 轉址 | 完整應用 | 完整 API | 200 |
| 額外網域，`active` | 主題感知短網址首頁（主題選擇器、語言切換） | 轉址 | GET/HEAD 308 回 canonical；其他方法 404 | 404 JSON | 200 |
| 額外網域，`pending`/`disabled` | 404 | 404 | 404 | 404 JSON | 200 |
| 未知 Host | 404 | 404 | 404 | 404 JSON | 200 |

`/assets/*`、`/favicon.ico`、`/favicon.svg`、`/health`、`/healthz` 不經 Host
檢查，讓錯誤頁與容器健康檢查在任何 Host 都能運作。

## 4. 資料模型

### `domains`

由 migration `0003_multi_domain` 新增：

| 欄位 | 意義 |
|------|------|
| `status` | `pending`、`active`、`disabled` |
| `source` | `system`（canonical）、`env`（`URLET_DOMAINS`）、`ui` |
| `updated_at` | 最後一次生命週期變更 |

`hostname` 以小寫儲存、去除結尾的點，可保留非預設 port。以 `NOCASE` 唯一
索引支援大小寫不敏感的查詢。既有的 `is_default` partial unique index 保證
只有一個預設網域。

### `urls`

- `domain_id INTEGER NOT NULL REFERENCES domains(id) ON DELETE RESTRICT`
- `UNIQUE (domain_id, slug)`，`slug` 保留既有的 `COLLATE NOCASE`
- 舊資料的 `NULL` 回填到預設網域
- soft-deleted 的資料仍保留 `(domain_id, slug)`，避免 slug 被劫持

Migration `0005_group_domains` 新增群組層級的網域政策：
`user_groups.default_domain_id`、`restrict_domains` 與 `group_domains`
allowlist，決定每位成員的有效預設與可選網域。

## 5. Migration `0003_multi_domain`

改名之前，migration 會先把 hostname 正規化到暫存映射，並合併正規化後相同的
資料列（大小寫、結尾的點、預設 port）。若群組中有預設列就保留預設列，否則保留
最小 id；`urls.domain_id` 會重新指向保留列，重複列刪除。這處理了 v2 安裝曾把
`URLET_BASE_URL` 從 `example.com:443` 改成 `example.com`、因而同時存在兩列的情況。

SQLite 無法直接移除 `urls.slug` 的 inline `UNIQUE`，因此用官方 12-step
流程重建資料表。因為 `clicks`、`url_tags`、`url_daily_stats` 以
`ON DELETE CASCADE` 參照 `urls`，重建必須在專用連線上、於 transaction
**外面**設定 `PRAGMA foreign_keys = OFF`。Migration runner 會區分一般
migration 與 rebuild migration，並在關閉連線前一定把
`PRAGMA foreign_keys = ON` 設回。commit 前 `PRAGMA foreign_key_check` 必須
沒有資料列。

升級前必須備份 SQLite 檔。此 migration 沒有 down migration；回滾方式是還原
備份與舊版 binary。測試會先建立 v2 資料庫，放入 URL、clicks、tags、
daily stats，再套用 `0003`，檢查列數、外鍵、回填的 domain、per-domain slug
唯一性，以及 `sqlite_sequence` 仍可正常遞增。

## 6. Host 解析

- 讀取 `Host` header。沒有 `Host`（HTTP/1.0 與測試）時視為 canonical；
  有 `Host` 但無法解碼或格式無效時視為未知網域。
- 正規化：trim、小寫、去結尾的點、去 `:80`/`:443`、保留其他 port；拒絕
  scheme、路徑與控制字元。
- Canonical host 可以是 IPv6 literal 或 Docker 風格的底線服務名稱；這類值
  透過 ASCII fallback 比對，額外網域仍必須是正常 hostname。Unicode canonical
  host 必須使用 punycode；無法解析的 Unicode `URLET_BASE_URL` 會在啟動時
  直接失敗，而不是靜默回 404。
- 本版本**不信任** `X-Forwarded-Host`。反向代理必須保留原始 `Host`
  （Caddy 預設會保留）。
- `AppState` 內的 `DomainRegistry` 快照提供 hostname/id 查詢、active host
  清單與 default id。Bootstrap 時載入，admin 變更後 reload；直接改資料庫的
  變更會在下次重啟才生效。
- Domain gate 在 locale 之後、CSRF 之前執行，將 `HostContext` extension
  （canonical 或 active domain）交給 handler。
- 未知、pending、disabled 的 Host 不會進到轉址 handler。

## 7. 建立與顯示短網址

- `CreateInput` 與 `POST /api/v1/urls` 接受選用 `domain` hostname；省略時由
  網域政策解析為呼叫者所屬群組的預設網域（群組未設定，或設定的網域已非
  active 時，回退到全站預設）。目標必須是 `active` 且被呼叫者的政策允許；
  `pending`、`disabled` 一律拒絕。
- 公開建立路徑 `POST /` 與 `POST /api/v1/public/urls` 也接受 `domain`；表單
  會在轉回頁面的查詢字串帶回解析後的 hostname，API 則在 `short_url` 之外
  回傳 `domain`。
- 所有 slug 檢查、自動產生重試、更新檢查都以選定網域為範圍。衝突訊息會帶
  hostname。
- `PATCH /api/v1/urls/{id}` 接受 `domain`；搬移時在目標網域重新檢查 slug，
  衝突回 `409`。
- `UrlDto` 新增 `domain`；`short_url` 為 `{scheme}://{hostname}/{slug}`，使用
  全站 scheme。
- HTML 建立與編輯表單顯示呼叫者政策允許的網域下拉選單；dashboard 與 detail
  頁的「開啟」連結改用 `short_url`，不再用相對路徑。
- `GET /api/v1/domains` 依呼叫者所屬群組的網域政策過濾，回傳可選的 active
  網域；`is_default` 代表群組的有效預設，不是全站預設。
- `GET /api/v1/public/urls/{slug}?domain=<hostname>`：省略 `domain` 時查預設
  網域，保持舊呼叫相容；回應新增 `domain`。

## 8. 管理

- 新增 `/app/admin/domains` 頁面與 `/api/v1/admin/domains` endpoints。
- 動作：新增（`pending`）、啟用、停用、設為預設、刪除。
- 規則：
  - canonical（`source = system`）永遠 active，不可停用或刪除。若 `URLET_BASE_URL` 改變，bootstrap 會建立新的 canonical row；若舊 system row 是預設，會把預設轉移過去，並把舊 system row 降級為 `ui`。舊連結會繼續解析，直到管理員停用該網域；
  - `pending`/`disabled` 不可設為預設；
  - 目前預設不可停用或刪除，必須先把另一個 active 網域設為預設；
  - `source = env` 不可從 UI 刪除（會被重新 seed），但可停用；把 hostname 從 `URLET_DOMAINS` 移除後，下次啟動會 detach 成 `source = ui`，之後即可正常停用或刪除。`system` 網域則不可停用、也不可刪除；
  - `ui` 網域只有在完全沒有連結（含 soft-deleted）時才能刪除。
- 每次變更都寫 audit（`domain.created`、`domain.updated`、
  `domain.deleted`、`domain.default_changed`）並 reload registry。

## 9. 安全

- Session cookie 保持 host-only；登入與應用只存在 canonical host。
- Unlock token 與 cookie 名稱同時綁定 hostname 與 slug，讓
  `a.example.com/promo` 的 token 不能解鎖 `b.example.com/promo`。
- 點擊紀錄的 referrer 自我排除清單包含所有 active hostname（加上 canonical、
  `localhost`、`127.0.0.1`）。
- Unlock rate-limit key 包含 domain。
- 轉址的 `Location` 一律來自儲存的目標或設定的 canonical base URL，絕不用
  請求的 `Host` 組裝。
- 本版本的 captcha 流程仍只在 canonical；未來在額外網域做 white-label 時
  需要 per-domain site key。
- 既有的 CSRF same-origin 檢查已接受與請求 `Host` 相符的
  `Origin`/`Referer`，涵蓋額外網域上的 unlock 表單。

## 10. 部署

- `URLET_BASE_URL` 仍是 canonical host，也是唯一的 scheme 來源。
- `URLET_DOMAINS=go.example.com,links.example.com` 用來 seed 額外網域。Seed
  會插入為 `active`、`source = env`、已驗證；不會更新既有資料。把 hostname
  在 `URLET_DOMAINS` 有設定時，把 hostname 從環境變數移除後，下次啟動會 detach
  成 `source = ui`（保留狀態、連結與預設），不會自動刪除。
- 每個新增網域都需要 DNS 指向此實例，以及對應的 Caddy site block（自動
  TLS）。若網域不是由 `URLET_DOMAINS` 管理，新增後必須 reload Caddy。
- 所有網域必須和 `URLET_BASE_URL` 使用相同 http/https scheme。Canonical 是
  https 時，無法表示只支援 http 的額外網域。
- Registry 是 in-process；這仍是單一實例的 SQLite 部署。

## 11. 測試

| 區域 | 案例 |
|------|------|
| Migration | v2 seeded DB → 0003；clicks/tags/stats 保留；FK check 空；NULL 回填；新插入 id |
| Redirect | canonical/active/pending/disabled/unknown host；同 slug 兩網域；`/app` 308；`/api` 404；`/health` 任何 host |
| API | create 帶／不帶 domain；invalid/disabled domain；slug 衝突；PATCH 搬移；public metadata `?domain=` |
| Admin | 新增/啟用/停用/預設/刪除；canonical 與 default 保護；source 規則；audit |
| Security | unlock cookie A 不解 B；`X-Forwarded-Host` 被忽略；open redirect |
| UI | 網域下拉選單；dashboard/detail 開啟正確 host；三語 |

## 12. 不在本次範圍（P2）

- DNS TXT 所有權驗證與自動啟用。
- Per-domain 品牌、主題或設定。
- 個別使用者的網域例外。群組層級的預設與 allowlist 已由 migration
  `0005_group_domains` 實作（`user_groups.default_domain_id`、
  `restrict_domains`、`group_domains`）；只有 per-user 覆寫不在範圍內。
- 額外網域上的完整 white-label 應用（登入/儀表板/API）。
- 由 admin UI 動態產生 Caddy 設定。
- Per-domain scheme 與 per-domain captcha key。
