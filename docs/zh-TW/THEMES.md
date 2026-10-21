**繁體中文** | [English](../THEMES.md)

# 佈景主題

urlet 在單一 binary 內建六套視覺主題。主題是**程式碼**：`templates/themes/<key>/`
下的 Askama 模板目錄與一個內嵌 CSS 檔。執行期不上傳主題，也不會用請求提供的路徑
載入模板。

| Key | 名稱 | 說明 |
|-----|------|------|
| `verdant` | Verdant Systems | v1.2 的正式設計，也是基底模板與回退；支援持久化的明暗控制 |
| `aurora` | Aurora Glass | 極光漸層與固定側欄的深色玻璃指揮中心；僅深色 |
| `paper` | Paper Ledger | 編輯風紙本帳冊；跟隨 `prefers-color-scheme` |
| `signal` | Signal | 粗獷訊號看板；跟隨 `prefers-color-scheme` |
| `gilded` | Gilded Circuit | 象牙白、祖母綠與香檳金的裝飾藝術；跟隨 `prefers-color-scheme` |
| `swiss` | Swiss Telemetry | 編號區塊與細線的瑞士網格；跟隨 `prefers-color-scheme` |

五個非預設主題來自已關閉的 PR #5、#6、#7、#9、#10，並已移植到 v1.2 頁面模型，
不是凍結在當時的功能集。

## 管理員操作

1. 開啟 **管理 → 主題**（`/app/admin/themes`）。
2. 勾選要提供的主題。至少要保留一個啟用主題。
3. 選擇**預設**主題；預設必須啟用，沒有 cookie 的訪客會看到它。
4. 選擇**誰可以變更自己的主題**：
   - `所有使用者`：已登入成員可以切換（預設）。
   - `僅管理員`：管理員仍可預覽；成員看不到切換器，直接切換回 `403`。
5. **預覽**只會套用到目前的管理員，之後回到主題頁。
6. 儲存後立即生效，不需重啟。

更改政策只控制誰可以**變更**選擇。已經持有啟用主題 cookie 的瀏覽器會繼續顯示該主題，
直到使用者切換、主題被停用或 cookie 過期。若要強制所有人回到同一外觀，請停用其他主題
（或更改預設並在反向代理清除 cookie）。

同樣的操作可透過 `GET/PUT /api/v1/admin/themes` 完成；JSON API 接受
`enabled`、`default`、`switching`，省略的欄位維持原值。

## 訪客主題的決定順序

1. `urlet_theme` cookie，且指向已啟用主題。
2. 否則使用管理員設定的預設。
3. cookie 指向已停用或未知主題時忽略，使用預設。

切換器是原生 `<details>`，沒有 JavaScript 也能使用。cookie 為 `HttpOnly`、
`SameSite=Lax`、一年，且在設定安全 cookie 時加上 `Secure`。主題是外觀偏好，
不是每位使用者的資料庫欄位；新瀏覽器會從預設開始。

## 模板與回退

根目錄的 `templates/` **同時**是 Verdant 主題與回退。主題目錄可覆寫任一頁面：

```
templates/
├── base.html                 # Verdant 基底／回退
├── app_base.html
├── dashboard.html
├── ...
├── content/                  # v1.2 共用頁面內容（macro）
│   ├── dashboard.html
│   └── admin/...
└── themes/
    ├── aurora/
    │   ├── base.html
    │   ├── app_base.html
    │   ├── landing.html
    │   ├── dashboard.html     # 包裝 content/dashboard.html 的薄模板
    │   └── ...
    ├── paper/
    └── ...
```

大量 v1.2 狀態（配額、網域、群組、邀請碼、設定）的頁面透過
`templates/content/` 的 Askama macro 共用。主題覆寫自己的外框與公開／登入頁，
再把共用內容放進自己的殼；這讓每個主題在產品長出新功能時仍能完整運作，代價是
舊設計在密集的管理頁面會比較接近基底內容。共用的
`src/assets/theme-ui.css` 會補上舊樣式表不認得的 class，並使用 `:where()`
讓原設計在有定義時永遠優先。

模板由 `src/routes/pages/themed.rs` 的 `themed_page!` 在編譯期選擇。每個頁面
context 加上 `.themed()`，macro 會為每個主題產生一個 Deref 到同一 context 的
小包裝，因此 Askama 的編譯期欄位檢查仍套用到每個主題。

## 新增主題

1. 在 `src/services/theme.rs` 的 `ThemeKey` 加入變體（key、名稱 key、說明 key、
   CSS URL、是否僅深色）。
2. 將 CSS 放到 `src/assets/themes/<key>.css`，在 `src/assets.rs` 與
   `/assets/themes/{key}` 路由註冊。
3. 在三個 `src/i18n/*.json` 加入目錄字串。
4. 建立 `templates/themes/<key>/`，至少包含 `base.html`、`app_base.html`、
   `landing.html`、登入／註冊／連結錯誤頁、管理 tabs 與應用頁面；密集頁面可重用
   content macro。
5. 在每個 `themed_page!` 呼叫加入 `templates/themes/<key>/...` 路徑。
6. 將主題加入 `ALL`、migration 種子與管理測試；既有資料庫在啟動時也可依賴
   `db::themes::ensure_defaults` 補上。
7. 執行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、
   `cargo test` 與 `scripts/smoke.sh`。

## 刻意限制

- 不做主題編輯器或上傳：主題編譯進 binary，維持自包含與
  `default-src 'self'` CSP。
- 不做逐帳號權限：政策是站台層級（`all` 或 `admin`）。既有使用者群組的
  功能開關處理資料能力，不處理外觀。
- 五個移植的設計不提供手動明暗切換，跟隨作業系統；Aurora 依設計僅有深色。
