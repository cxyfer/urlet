# SNS 預覽實作計畫

目標：登入使用者可替自己的每條短網址設定公開 SNS 卡片標題、內容與圖片，圖片支援 HTTPS 網址及 PNG／JPEG／GIF／WebP 上傳。
架構：保留既有短網址生命週期與轉址 handler，每條連結至多新增一筆 url_previews。service 負責驗證，db 負責 SQL，routes 負責表單與 API。圖片資料存於 SQLite，替換時更換不透明網址 key。只有已知社群爬蟲取得 metadata HTML，一般瀏覽器沿用直接轉址、中繼頁及密碼頁。
技術：Rust／Axum／SQLx／Askama、SQLite、既有 HTML 表單與三語目錄；image 0.25.6 精確鎖定版本，image-webp 將最低 Rust 版本提升至 1.80.1，使用受限解碼。
基準與權威：2026-10-23 使用者要求、AGENTS.md、SPEC 的 FR-SPLASH 與 FR-PUB、PLAN、db/mod.rs、redirect.rs、ua.rs、security.rs 均已閱讀；沒有阻擋開發的未決需求。使用者明確授權計畫後直接開發，並要求獨立審核及 PR。
相容邊界：不修改已發布 migration；預覽不公開管理用標題、備註或目標網址；未設定自訂預覽及真人瀏覽器行為維持原狀。生命週期先於預覽。自訂 metadata 與圖片明確公開，即使連結有密碼。不代抓外站、不提供訪客上傳，不發布版本、不部署、不合併。
驗證：cargo fmt --check、cargo clippy --all-targets -- -D warnings、cargo test、bash scripts/smoke.sh、bash scripts/merge-check.sh、實際 HTTP multipart／metadata／圖片驗證與獨立 subagent 審核。

## 實作前確認

需求已可執行：標題、內容與兩種圖片來源已確認；預設標題只取站台名稱，不抓取目標頁面；清空預覽即停用。現有轉址沒有可設定的 metadata，僅改設定或文件無法滿足需求，故須修改持久化、驗證、HTTP 與詳情頁。
沿用既有擁有者與生命週期、middleware、詳情頁與共用模板。獨立選填資料表避免一般短網址列表讀取圖片 BLOB；preview service 為單一政策來源，不新增平行儲存系統。
統計用 is_bot 包含 curl、監控及搜尋引擎，不適合作預覽條件；在既有 UA 模組新增限定 SNS 的辨識。SQLite 仍是唯一新增持久化來源。
方案比較：僅回傳 redirect 無法提供自訂 metadata；全部請求回傳 HTML 會改變真人轉址；僅向社群爬蟲回傳預覽可保留真人行為。只支援圖片網址不能滿足上傳需求；另存檔案會新增備份與磁碟掛載契約。
複雜度：以獨立且有界的 db／service／route 模組及共用表單，避免持續擴大既有巨大 handler。既有轉址、中繼頁與密碼流程保留為各場景的權威。
執行條件：以匯出的本地 origin/master 4798f46 為基準，直接在本回合開發；未要求嚴格 TDD，以有意義的 integration tests 與完整檢查驗證。初次 Git 管理資料寫入遭自動審核拒絕，故先於 /tmp/urlet-social-previews 實作與驗證。確認遠端身分、ADMIN 權限、乾淨主工作樹及排除路徑後，已獲准在 .worktrees/social-previews 建立 feat/social-previews，基準 cb21e88，使用獨立 target/ 驗證後整合。

## 工作項目

1. 資料與驗證：新增 migrations/0016_url_previews.sql 並註冊；新增 db/url_previews.rs，提供不含圖片 BLOB 的讀取、擁有者重新檢查、原子寫入／清除與圖片查詢。service/url_preview.rs 驗證標題 200 字、內容 1000 字、無帳密控制字元的完整 HTTPS 圖片網址 2048 字；上傳上限 2 MiB、4096 × 4096 像素，blocking worker 受限解碼，明確保留／替換／移除圖片。以驗證與預覽 integration tests 檢查。
2. HTTP 與介面：新增 routes/previews.rs，提供 GET／PATCH／DELETE /api/v1/urls/{id}/preview、POST /api/v1/urls/{id}/preview/image、POST /app/urls/{id}/preview。擁有者認證、CSRF、請求大小限制及無效修改不得覆寫資料。六主題詳情頁沿用共用表單，三語標籤、公開／快取提示、可理解錯誤、保留文字的重試頁與既有成功訊息。
3. 爬蟲與圖片：在既有 ua.rs 加入限定社群辨識；有效且已設定預覽的連結在密碼流程之前對 SNS 爬蟲回傳獨立跳脫模板，僅含公開預覽與短網址。回應 no-store、Vary: User-Agent；預覽不計點擊。圖片由 /assets/link-previews/{opaque-key} 提供，使用驗證後 MIME、nosniff、no-store，遵循連結生命週期；替換與移除使舊 key 失效。
4. 測試與文件：涵蓋真人／已知與未知機器人、密碼／過期／停用／刪除／點擊上限、HTML 跳脫、權限／CSRF／大小、網址／上傳切換、清除、舊 key、失敗原子性、重開資料庫、六主題三語系。更新 SPEC、API、PLAN 及繁體鏡像與 README，說明公開性、限制與第三方快取。
5. 驗證與整合：完成所有專案檢查、實際 HTTP，再由獨立 reviewer 對來源與原基準審查；修正重要問題後重跑相關檢查。備妥 patch 與 PR 標題／內文；使用已獲准的專用 worktree 與 feat/social-previews，從確認基準套用變更、以既有 gitmoji 風格提交、推送並建立對 cxyfer/urlet:master 的 PR，不合併。

## 風險與回復

SNS User-Agent 與平台快取會變動，本地 HTTP 測試只證明伺服器回應，不等同實際 Discord 卡片驗收；不保證特定版型或立即更新。圖片使 SQLite 增大（每連結至多 2 MiB），沿用資料庫備份。停用功能時可保留 additive migration，不編輯或刪除已套用 migration、既有連結；不記錄憑證值。獨立審核後新增全域兩個解碼 permit，64 MiB 為解碼 buffer 預算而非程序 RSS 硬上限，額外工作記憶體須另計。

獨立審核補強：兩個全域解碼 permit；解碼前檢查 WebP RIFF、畫布及巢狀 VP8／VP8L／ANMF 尺寸，並以 GIF descriptor 掃描確保所有影格位於受限畫布內。64 MiB 解碼 buffer 預算不等於程序 RSS 上限。為 image-webp 將宣告最低 Rust 版本從 1.80 提升至 1.80.1。
