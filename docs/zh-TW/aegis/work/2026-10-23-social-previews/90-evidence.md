# 驗證證據

驗證專用 worktree：.worktrees/social-previews，分支 feat/social-previews，已整合 master 5a0646b（追蹤參數清理）。兩個規格章節皆保留，預覽編為 4.18。必要檢查在最後的解碼、刪除生命週期與登入逾期修正後執行，使用獨立 target/ 與 CARGO_INCREMENTAL=0。主機 registry 唯讀，故使用可寫的 Cargo registry 副本；未共用 Cargo 建置目錄或硬連結建置快取。

- cargo fmt --check：通過。
- cargo clippy --locked --offline --all-targets -- -D warnings：通過。
- cargo test --locked --offline：558 成功、0 失敗、2 個既有測試略過（實際供應商 HTTPS 與模板巨集文件）。
- cargo build --locked --offline：通過。
- bash scripts/smoke.sh：177 成功、0 失敗。
- bash scripts/merge-check.sh：42 成功、0 失敗。
- node --check src/assets/app.js 及 git diff --check：通過。
- 13 個新整合測試涵蓋有效／損壞圖片、惡意 GIF 第一／第二影格、擁有權／CSRF／限制、metadata 跳脫與隱私、圖片替換、持久化、六主題三語呈現、自訂網域與瀏覽器流程。連結／帳號軟刪除會移除預覽資料與圖片，保留其他擁有者與可恢復生命週期狀態的預覽。未登入表單會導向登入並記住連結詳情返回路徑，不寫入資料。兩個 WebP 單元測試涵蓋隱藏超大影格及截斷 RIFF。
- 原始功能原始碼的瀏覽器驗證：六主題、四種圖片來源的 computed display 與 input disabled 狀態正確；HTTPS 網址與 WebP 上傳提交皆成功。390px 繁中 viewport 的文件寬度為 390px，沒有水平溢出。實際 Discordbot HTTP 回應包含儲存的 OG／Twitter 標題、內容與上傳圖片網址。
- 獨立實作審核者複查後無剩餘可採取問題；其獨立解碼 harness 接受九種有效 WebP 與正常 GIF 動畫，拒絕 GIF 第一／第二影格越界。獨立 PR 審核發現軟刪除後保留圖片 BLOB，以及 HTML handler 回傳 JSON 認證錯誤；修正與整合差異經複查，未見其他可採取問題。審核者未另跑完整 Cargo 套件或驗證第三方 SNS 實際呈現。
- GitHub API 確認 cxyfer/urlet、ADMIN 權限及 master 5a0646b2bf2959e1f16b3331cd694eb97621600f。已建立授權的專用 worktree、分支與 PR #35。未合併到 master 或部署。

日誌與瀏覽器截圖為本地暫存產物，不屬產品檔案。第三方卡片版型、圖片接受情況與快取更新尚未驗收。宣告最低 Rust 版本為 1.80.1；檢查使用已安裝工具鏈，未使用 MSRV 工具鏈。本次關閉增量編譯以減少建置產物，代價是後續重編可能較慢。SQLite 可重用刪除圖片釋放的頁面，但資料庫檔案不會自動縮小。
