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

## 管理員圖片政策驗證

新增 preview_image_policy=none/url/upload/both 後，在專用 worktree 重新檢查通過：格式、Clippy warnings denied、562 個 Cargo 測試（0 失敗、2 個既有略過）、建置、177 個 smoke、42 個群組／網域檢查。17 個預覽整合測試全數通過。本次後續未修改既有 checksum-protected migration；啟動時僅補入缺少的新設定。

瀏覽器證據：管理員表單成功儲存 none，擁有者編輯器在四模式只提供允許的來源，啟用來源欄位的顯示／disabled 狀態正確，390px 繁中畫面無水平溢出。截圖與檢查結果位於 /tmp/urlet-preview-policy-ui。獨立 HTTP／SQLite harness 驗證四政策下的 API／表單寫入、OG metadata、圖片狀態／no-store、DTO，以及切換後儲存圖片資料不變。沒有剩餘可採取審核問題。既有軟刪除討論 PRRT_kwDOVLtob86vW_AP 已以 a244478 證據回覆並 resolve；實際第三方 SNS 快取／呈現與 MSRV 仍未驗證。

## 訪客 allowlist 整合驗證

將 master b36a1e3（訪客目標網址 allowlist，PR #34）整合至功能分支。僅三份語系 JSON 尾端新增 key 發生衝突；獨立審核比對每份目錄的 877 個 key 與雙方版本，確認完整聯集、無重複 key 或覆寫值。設定／表單同時保留兩項獨立政策；訪客 allowlist 的授權不影響已登入的預覽編輯及預覽輸出。獨立審核未發現可採取問題，未另跑 Cargo。

整合後重新驗證通過：cargo fmt --check、Clippy warnings denied、577 個 Cargo 測試（0 失敗、2 個既有略過）、建置、192 個實際 binary smoke 檢查及 42 個群組／網域檢查。預覽測試仍為 17 個。輸出位於 /tmp/urlet-preview-b36-integration.log；必要檢查使用專用 worktree 及其獨立 target 目錄。
