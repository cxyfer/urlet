# 檢查點

已實作 additive migration 0016、SQLite 預覽與圖片 BLOB、驗證、擁有者表單／API、六主題編輯器、三語目錄、OG／Twitter 回應及受生命週期限制的公開圖片。獨立審核發現的 WebP 內層尺寸、GIF 影格邊界、全域解碼併發、圖片負向快取、CSS hidden 優先序、MSRV、稽核測試舊紀錄假陽性、軟刪除圖片清理與 HTML 登入恢復均已修正。

整合 master 5a0646b 後的最後完整檢查通過：558 個測試成功、2 個既有測試略過，177 個 smoke 檢查、42 個群組／網域檢查，以及格式、Clippy 與建置。瀏覽器已驗證六主題的四種圖片來源狀態、網址／上傳提交，以及 390px 繁中畫面無水平溢出。

專用 worktree 為 .worktrees/social-previews，分支 feat/social-previews，PR #35 以 master 為基準。保留較新的追蹤清理功能與管理分頁修正，解決兩語規格章節衝突。獨立實作與 PR 審核已檢查修正，沒有剩餘可採取問題。分支依授權的 commit／push／PR 流程整合，範圍不含合併到 master 或部署。
