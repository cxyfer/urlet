# 檢查點

已實作 additive migration 0016、SQLite 預覽與圖片 BLOB、驗證、擁有者表單／API、六主題編輯器、三語目錄、OG／Twitter 回應及受生命週期限制的公開圖片。獨立審核發現的 WebP 內層尺寸、GIF 影格邊界、全域解碼併發、圖片負向快取、CSS hidden 優先序、MSRV 與稽核測試舊紀錄假陽性均已修正。

GIF descriptor 補強後的最後完整檢查通過：545 個測試成功、2 個既有測試略過，177 個 smoke 檢查、42 個群組／網域檢查，以及格式與 Clippy。瀏覽器已驗證六主題的四種圖片來源狀態、網址／上傳提交，以及 390px 繁中畫面無水平溢出。

Git 整合已獲准。專用 worktree 為 .worktrees/social-previews，分支 feat/social-previews，基準 cb21e88。原始碼匯出的差異檔乾淨套用，保留上游管理分頁修正；全新獨立 target/ 的必要檢查均通過，接續 commit、push、PR 與獨立 PR 審核流程。主 checkout 維持乾淨。
