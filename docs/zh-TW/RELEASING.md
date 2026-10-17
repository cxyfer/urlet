# uwurls 發布流程

[English](../../RELEASING.md) | **繁體中文**

這份文件是專案的發布規則。建立版本標籤前請先讀過；自動化流程位於
[`.github/workflows/release-image.yml`](../../.github/workflows/release-image.yml)。

## 一次發布會做什麼

一次發布＝推送一個符合 `vMAJOR.MINOR.PATCH` 的 git 標籤（可帶 `-PRERELEASE` 後綴）。
推送後會執行一個 GitHub Actions workflow：

1. 驗證標籤與 `Cargo.toml` 的版本一致，
2. 建置 `linux/amd64` 與 `linux/arm64`，
3. 把映像推送到 GitHub Container Registry。

它**不會**自動修改版本、執行測試或建立 GitHub Release；這些步驟刻意留給人工處理
（見[規則](#規則)）。

## 映像標籤

| 發布 | 推送的標籤 | 說明 |
|------|------------|------|
| `v1.2.3` | `1.2.3`、`1.2`、`1`、`latest` | 正式版本 |
| `v1.2.3-rc.1` | `1.2.3-rc.1` | 預發布版：不會有浮動標籤 |
| 手動重建（預設） | `1.2.3` | 只重建精確版本 |
| 手動重建 + 勾選「update moving tags」 | `1.2.3`、`1.2`、`1`、`latest` | 僅正式版本適用 |

完整映像名稱為 `ghcr.io/cxyfer/uwurls`，由 `github.repository` 推導；fork 會發布到自己的命名空間。

## 規則

1. **標籤必須等於 `Cargo.toml` 的版本。** `v1.2.3` 要求 `version = "1.2.3"`，否則 workflow
   會在建置前直接失敗。
2. **升版後必須更新 `Cargo.lock`。** [Dockerfile](../../Dockerfile) 使用
   `cargo build --locked`，根套件版本不同步會讓映像建置失敗：
   ```bash
   cargo update -p uwurls
   cargo metadata --locked --format-version 1 >/dev/null
   ```
3. **只能標記已合併的 commit。** workflow 檔案必須存在於被標記的 commit 上，因此請先推送
   `master`，再推送標籤。
4. **不要移動或重複使用已發布的標籤。** 標籤是映像的不可變身分；修正請發布新的 patch 版本。
5. **不要對舊標籤勾選浮動標籤重建。** 那會把 `latest`（以及 `X`、`X.Y`）指回舊版本。舊標籤請
   使用預設值（只重建精確版本）。
6. **預發布版永遠不會有 `latest`。** 請使用 `-rc.1`、`-beta.1`、`-alpha.1` 之類的後綴。
7. **GitHub Release 由人工發布。** 映像發布完成後，自行撰寫 release notes。
8. **GHCR 可見性是儲存庫設定。** 私有儲存庫會產生私有套件；請執行
   `docker login ghcr.io`（token 需要 `read:packages`），或把套件設為公開。
9. **[docker-compose.yml](../../docker-compose.yml) 的 `image:` 只是本機建置標籤。** 更新它
   只是為了可讀性；正式發布的映像一律來自 workflow。

## 發布檢查清單

### 1. 事前檢查

```bash
git switch master
git pull --ff-only
git status --short          # 必須是空的

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
bash scripts/smoke.sh       # 建議執行：會建置並實際操作二進位檔
```

目前沒有會自動執行這些檢查的 CI，因此這一步就是品質關卡。

### 2. 升版

編輯 [`Cargo.toml`](../../Cargo.toml)：

```toml
version = "1.2.3"
```

更新 lockfile 並確認它是最新的：

```bash
cargo update -p uwurls
cargo metadata --locked --format-version 1 >/dev/null
```

提交兩個檔案：

```bash
git add Cargo.toml Cargo.lock
git commit -m "🔖 release: v1.2.3"
git push origin master
```

本專案使用 gitmoji 風格的主旨；`🔖 release: vX.Y.Z` 與現有歷史一致。

### 3. 建立並推送標籤

```bash
git tag -a v1.2.3 -m v1.2.3
git push origin v1.2.3
```

推送標籤就會啟動 workflow。

### 4. 驗證映像

```bash
gh run list --workflow=release-image.yml --limit 5
gh run watch <run-id>

docker login ghcr.io      # 只有私有套件需要
docker buildx imagetools inspect ghcr.io/cxyfer/uwurls:1.2.3
docker run --rm ghcr.io/cxyfer/uwurls:1.2.3 version   # 印出 "uwurls 1.2.3"
```

`imagetools inspect` 必須同時列出 `linux/amd64` 與 `linux/arm64`。

### 5. 發布 GitHub Release

*Releases → Draft a new release → 選擇既有標籤 → 撰寫說明 → 發布。*
此時映像已經可以使用。

### 6. 選用：允許匿名拉取

若希望匿名 `docker compose pull` 可以運作，請到套件頁面設定
*Package settings → Danger zone → Change visibility → Public*。

## 手動重建

*Actions → Release container image → Run workflow*：

- `tag`：要重建的既有標籤（例如 `v1.2.3`）。
- `update_moving_tags`：除非你重建的是**最新正式版本**，且希望 `X`、`X.Y`、`latest` 跟著
  新 digest 移動，否則請保持**關閉**。

常見用途：修補基底映像的 CVE，或 runner/toolchain 變更，但不需要新的應用程式版本。

## 失敗處理

| 情況 | 處理方式 |
|------|----------|
| 版本不一致（validate job 失敗） | 刪除標籤（`git tag -d vX.Y.Z`、`git push origin :refs/tags/vX.Y.Z`），修正 `Cargo.toml`／`Cargo.lock` 並提交後重新建立標籤 |
| 建置失敗且尚未發布任何映像 | 在 `master` 修正，刪除標籤後重新建立 |
| 映像已發布但有問題 | 不要重用標籤。升 patch 版本重新發布；期間在部署端固定使用前一個版本 |
| 標籤在 workflow 存在前就已推送 | 用該標籤手動執行 workflow（關閉浮動標籤） |
| 無法拉取套件 | 執行 `docker login ghcr.io`，或把套件設為公開 |
| 需要前一個映像 | 設定 `UWURLS_IMAGE=ghcr.io/cxyfer/uwurls:<舊版本>` 並重建容器 |

## 快速複製指令

```bash
VERSION=1.2.3

sed -i "s/^version = \".*\"$/version = \"$VERSION\"/" Cargo.toml
cargo update -p uwurls
cargo metadata --locked --format-version 1 >/dev/null

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

git add Cargo.toml Cargo.lock
git commit -m "🔖 release: v$VERSION"
git push origin master
git tag -a "v$VERSION" -m "v$VERSION"
git push origin "v$VERSION"
```

查看目前版本：

```bash
grep '^version' Cargo.toml
```
