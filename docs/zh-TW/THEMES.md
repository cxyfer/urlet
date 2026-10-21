**繁體中文** | [English](../THEMES.md)

# 佈景主題

urlet 在單一 binary 內建六套視覺主題，管理員另可在執行期上傳「純樣式表」主題。
**內建主題**是程式碼：`templates/themes/<key>/` 下的 Askama 模板目錄與一個內嵌
CSS 檔。**上傳主題**則是存放於持久化 volume 的 CSS 檔；由於 Askama 模板在編譯期
固定，上傳主題沿用基礎頁面結構，只替換樣式表。

| Key | 名稱 | 說明 |
|-----|------|------|
| `verdant` | Verdant Systems | v1.2 的正式設計，也是基底模板與回退；支援明暗切換 |
| `aurora` | Aurora Glass | 極光漸層與固定側欄的深色玻璃指揮中心；僅深色 |
| `paper` | Paper Ledger | 編輯風紙本帳冊；跟隨 `prefers-color-scheme` |
| `signal` | Signal | 粗獷訊號看板；跟隨 `prefers-color-scheme` |
| `gilded` | Gilded Circuit | 象牙白、祖母綠與香檳金裝飾藝術；跟隨 `prefers-color-scheme` |
| `swiss` | Swiss Telemetry | 編號區塊的瑞士遙測網格；跟隨 `prefers-color-scheme` |
| `custom-…` | 自訂 | 由 **管理 → 主題** 上傳的樣式表，沿用 Verdant 模板 |

五個非預設的內建主題來自已關閉的 PR #5、#6、#7、#9、#10，並已移植到 v1.2
頁面模型，而非凍結在原本的功能集。

## 管理員操作流程

1. 開啟 **管理 → 主題**（`/app/admin/themes`）。
2. 上傳自訂樣式表：填寫名稱、選填說明，將 `.css` 檔拖曳到上傳區（或點擊選檔），
   並選擇**跟隨系統**、**僅淺色（純白）**或**僅深色（純黑）**。上傳後會立即
   出現在目錄中，預設為啟用。
3. 勾選要提供的主題。至少要保留一個啟用主題。
4. 選擇**預設**主題；預設必須啟用，沒有 cookie 的訪客會看到它。
5. 選擇**誰可以變更自己的主題**：
   - `所有使用者` — 已登入成員可切換（預設）。
   - `僅管理員` — 管理員仍保留切換器供預覽；成員看不到，直接切換會回 403。
6. **預覽**只會套用到目前的管理員，之後回到主題頁。
7. 儲存。選擇會立即生效，不需要重啟。

上傳主題可在編輯頁重新命名、調整外觀，或替換樣式表。**刪除**會移除目錄
資料列與 CSS 檔；目前是預設的主題不能刪除，請先選擇其他預設。原始檔名僅供
顯示；實際路徑由產生的 key 與內容雜湊決定。

政策在頁面渲染時強制執行，不只是隱藏切換器。設為**僅管理員**時，非管理員帳號與
未登入訪客一律顯示網站預設；既有 `urlet_theme` cookie（包含政策仍為
`所有使用者` 時設定的）會被忽略。管理員仍看得到切換器，並可用**預覽**檢視任何
主題（包含尚未啟用者），而不改變訪客看到的預設。預覽 cookie 生命週期為一小時。
一般切換器點選或**結束預覽**會離開預覽模式。

同樣的選擇可透過 `GET/PUT /api/v1/admin/themes` 完成；JSON API 接受
`enabled`、`default` 與 `switching`，省略的欄位維持原值。上傳本身是瀏覽器
multipart 表單，JSON API 只負責讀取與更新目錄，不是檔案上傳端點。`GET` 會以
`"custom": true` 與帶雜湊的 `css_url` 回傳上傳主題。

## 自訂主題的儲存位置

| 項目 | 位置 |
|------|------|
| 中介資料 | SQLite 的 `custom_themes` 表 |
| 目錄狀態 | `themes` 表中對應的資料列（key、啟用、預設、排序） |
| 樣式表 | `<URLET_THEMES_DIR>/<key>-<內容雜湊>.css` |
| 預設目錄 | `<URLET_DATA_DIR>/themes` |
| Docker | named volume `urlet-themes`，掛載於 `/data/themes` |

內容雜湊是 `/assets/custom-themes/{key}?v=<hash>` 的一部分，因此替換樣式表會
改變 URL，不會被一小時的資產快取遮蔽。中介資料與檔案都位於持久的 `/data`
volume；重啟後 registry 會從資料庫重建，上傳主題仍可選用。

上傳有明確限制：

- 單一 `.css` 檔、UTF-8、最大 256 KiB；
- 每個執行個體最多 32 個自訂主題；
- 完全不可出現外部 URL：`url(...)`、`image-set(...)` 與裸 `https://` 都算，
  且會先解碼 CSS 跳脫序列，`u\72l(` 無法繞過檢查；
- 不可使用 `@import`、`expression(...)` 或舊式 `behavior`；
- 以 `text/css` 與 `X-Content-Type-Options: nosniff` 提供，無法變成 HTML 文件。

上傳主題也會決定調色盤的行為：

- **跟隨系統**：保留一般明暗切換，訪客尚未選擇前跟隨作業系統。
- **僅淺色**、**僅深色**：由伺服器強制 `data-theme="light"`／`"dark"`、隱藏
  明暗切換，且瀏覽器儲存的偏好無法覆寫。這就是「純白」／「純黑」選項。

上傳的 CSS 會**接在** Verdant 樣式表之後提供，因此小幅的顏色或版面覆寫能與基礎
設計疊加，並在串接中勝出。上傳面板可下載 `/assets/app.css` 作為參考。自訂上傳
不會改變文件模板：它可以重新配色所有頁面，但不能新增標記或腳本。

## 訪客主題的決定順序

1. `urlet_theme` cookie，且指向已啟用主題（內建或上傳）。
2. 否則使用管理員設定的預設主題。
3. cookie 指向已停用或未知主題時忽略，使用預設。

切換器是原生 `<details>` 元素，沒有 JavaScript 也能運作。cookie 為 `HttpOnly`、
`SameSite=Lax`、一年，且在設定安全 cookie 時加上 `Secure`。主題是外觀偏好，
不是每位使用者的資料庫欄位；新瀏覽器會從預設開始。切換器的**網站預設**項目會
清除 cookie，重新跟隨管理員的預設。

與語言切換器相同，主題切換端點是 `GET`，只會寫入外觀 cookie，不會變更資料或
權限。跨站頂層導覽因此可以改變外觀，但不能取得存取權或變更帳號。

## 模板與回退

根目錄的 `templates/` **同時**是 Verdant 主題與回退。內建主題目錄可覆寫任一頁面：

```
templates/
├── base.html                 # Verdant 基底／回退
├── app_base.html
├── dashboard.html
├── ...
├── content/                  # 共享的 v1.2 頁面內容（macro）
│   ├── dashboard.html
│   └── admin/...
└── themes/
    ├── aurora/
    │   ├── base.html
    │   ├── app_base.html
    │   ├── landing.html
    │   ├── link_splash.html   # 公開中轉頁
    │   ├── dashboard.html     # content/dashboard.html 的薄包裝
    │   └── ...
    ├── paper/
    └── ...
```

帶有大量 v1.2 狀態的頁面（配額、網域、群組、邀請碼、設定）由
`templates/content/` 的 Askama macro 共享。主題覆寫自己的外框與公開／登入頁，
再把共用內容放進自己的殼；這讓每個主題在產品長出新功能時仍能完整運作，代價是
較舊的設計在密集的管理頁面會更接近基礎內容。共用的 `src/assets/theme-ui.css`
補上那些舊樣式表不認得的 class，並使用 `:where()` 讓原設計在有其意見時勝出。

模板由 `src/routes/pages/themed.rs` 的 `themed_page!` 在編譯期選擇。每個頁面
context 加上 `.themed()`，macro 會為每個主題產生一個 Deref 到同一 context 的小
包裝，因此 Askama 的編譯期欄位檢查仍套用到每個主題。上傳主題在編譯期選擇
Verdant 包裝，只替換樣式表 URL。

## 新增內建主題

1. 在 `src/services/theme.rs` 的 `ThemeKey` 加入變體（key、名稱 key、說明 key、
   CSS URL、僅深色旗標）。
2. 將 CSS 放到 `src/assets/themes/<key>.css`，在 `src/assets.rs` 與
   `/assets/themes/{key}` 路由註冊。
3. 在三個 `src/i18n/*.json` 加入目錄字串。
4. 建立 `templates/themes/<key>/`，至少包含 `base.html`、`app_base.html`、
   `landing.html`、登入／註冊／連結中轉頁／錯誤頁、管理 tabs 與應用頁面；密集頁面可重用
   content macro。
5. 在每個 `themed_page!` 呼叫加入 `templates/themes/<key>/...` 路徑。每個呼叫都要保留
   `Verdant` 項目，它也是頁面尚未列出新主題時的回退。
6. 將主題加入 `ALL`、migration 種子與管理測試；既有資料庫在啟動時也可依賴
   `db::themes::ensure_defaults` 補上。
7. 執行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、
   `cargo test` 與 `scripts/smoke.sh`。

## 刻意限制

- 上傳的是樣式表，不是模板。CSP 維持 `default-src 'self'`，binary 保持
  自包含；自訂主題不能新增標記或腳本。
- 只有管理員能上傳。沒有每位使用者的主題市集或個人上傳配額；全站目錄仍是唯一
  來源。
- 不從遠端抓取主題：伺服器不會下載樣式表，且除了 CSP 之外，外部 `url(...)` 與
  `@import` 也會被拒絕。
- 沒有細緻的個人權限：政策是全站 `all`／`admin`。既有的使用者群組功能開關負責
  各群組的資料能力，不負責外觀。
- 五個移植設計沒有手動明暗切換，跟隨作業系統；Aurora 依設計僅提供深色。
