# 驗證證據

驗證原始碼：/tmp/urlet-social-previews，基準 4798f46。最後檢查在 GIF／WebP／稽核修正後執行，使用獨立 target/ 與 CARGO_INCREMENTAL=0。主機 registry 唯讀，故使用可寫的 Cargo registry 副本；未共用 Cargo 建置目錄或硬連結建置快取。

- cargo fmt --check：通過。
- cargo clippy --locked --offline --all-targets -- -D warnings：通過。
- cargo test --locked --offline：543 成功、0 失敗、2 個既有測試略過（實際供應商 HTTPS 與模板巨集文件）。
- cargo build --locked --offline：通過。
- bash scripts/smoke.sh：177 成功、0 失敗。
- bash scripts/merge-check.sh：42 成功、0 失敗。
- node --check src/assets/app.js：通過。
- 11 個新整合測試涵蓋有效／損壞圖片、惡意 GIF 第一／第二影格、擁有權／CSRF／限制、metadata 跳脫與隱私、圖片替換、持久化、六主題三語呈現、自訂網域與既有瀏覽器流程。兩個 WebP 單元測試涵蓋隱藏超大影格及截斷 RIFF。
- 瀏覽器：六主題、四種圖片來源的 computed display 與 input disabled 狀態正確；HTTPS 網址與 WebP 上傳提交皆成功。390px 繁中 viewport 的文件寬度為 390px，沒有水平溢出。實際 Discordbot HTTP 回應包含儲存的 OG／Twitter 標題、內容與上傳圖片網址。
- 獨立唯讀審核者複查後無剩餘可採取問題；其獨立解碼 harness 接受九種有效 WebP 與正常 GIF 動畫，拒絕 GIF 第一／第二影格越界。審核者未另跑 Cargo 套件或驗證第三方 SNS 實際呈現。
- GitHub API 確認 cxyfer/urlet、ADMIN 權限及 master cb21e8836f3ed340abeb67a395c74284012ea03a。功能差異檔對 4798f46 與 cb21e88 都通過 git apply --check；主要工作樹乾淨。

日誌與瀏覽器截圖為本地暫存產物，不屬產品檔案。第三方卡片版型、圖片接受情況與快取更新尚未驗收。宣告最低 Rust 版本為 1.80.1；檢查使用已安裝工具鏈，未使用 MSRV 工具鏈。

最後整合驗證：.worktrees/social-previews、feat/social-previews，基準 cb21e88。全新 target/ 通過格式、Clippy、545 個 Cargo 測試（2 個既有略過）、cargo build、177 個 smoke、42 個群組／網域、node --check 及 git diff --check。多出的兩個測試來自較新的上游基準。確認遠端與權限後，已授權的 worktree 建立操作成功。本次關閉增量編譯以減少建置產物，代價是後續重編可能較慢。
