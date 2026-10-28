# urlet 發布流程

[English](../../RELEASING.md) | **繁體中文**

這份文件是專案的發布規則。建立版本標籤前請先讀過；自動化流程位於
[`.github/workflows/release-image.yml`](../../.github/workflows/release-image.yml)。

## 目錄

- [一次發布會做什麼](#一次發布會做什麼)
- [映像標籤](#映像標籤)
- [規則](#規則)
- [發布檢查清單](#發布檢查清單)
- [手動重建](#手動重建)
- [清理重建留下的版本](#清理重建留下的版本)
- [失敗處理](#失敗處理)
- [快速複製指令](#快速複製指令)

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
| `v1.2.3`（最新正式版） | `v1.2.3`、`v1.2`、`v1`、`latest` | 正式版本 |
| `v1.2.4`（比最新版舊） | `v1.2.4` | 只推精確標籤；浮動標籤留在最新正式版 |
| `v1.2.3-rc.1` | `v1.2.3-rc.1` | 預發布版：不會有浮動標籤 |
| 手動重建（預設） | `v1.2.3` | 只重建精確版本 |
| 手動重建 + 勾選「update moving tags」 | `v1.2.3`、`v1.2`、`v1`、`latest` | 僅當 `v1.2.3` 是最新正式版時適用 |

映像標籤與 git 標籤一致，包含 `v` 前綴；`latest` 是唯一不帶前綴的標籤。1.3.0 之前的版本使用
裸版號（`1.2.0`、`1.2`、`1`），這些標籤不會再移動——請改用 `vX.Y.Z` 或 `latest`。

完整映像名稱為 `ghcr.io/cxyfer/urlet`，由 `github.repository` 推導；fork 會發布到自己的命名空間。

## 規則

1. **標籤必須等於 `Cargo.toml` 的版本。** `v1.2.3` 要求 `version = "1.2.3"`，否則 workflow
   會在建置前直接失敗。
2. **三個檔案要一起升版。** 版本號存在於 `Cargo.toml`、`Cargo.lock` 與
   [docker-compose.yml](../../docker-compose.yml) 的 `image:` 標籤，三處都必須是發布版本。
   改完 `Cargo.toml` 後要更新 lockfile——[Dockerfile](../../Dockerfile) 使用
   `cargo build --locked`，根套件版本不同步會讓映像建置失敗：
   ```bash
   cargo update -p urlet
   cargo metadata --locked --format-version 1 >/dev/null
   ```
   內嵌的 API 參考也帶有同一個版本（`info.version` 與 health 範例），因此要在同一個 commit
   重新產生合約。CI 的 `contracts` job 會在合約過期時失敗：
   ```bash
   python3 scripts/build-openapi.py
   ```
3. **只能標記已合併的 commit。** workflow 檔案必須存在於被標記的 commit 上，因此請先推送
   `master`，再推送標籤。
4. **不要移動或重複使用已發布的標籤。** 標籤是映像的不可變身分；修正請發布新的 patch 版本。
5. **只有最新正式版會移動 `vX`、`vX.Y`、`latest`。** 比最新版舊的維護／backport 版本只會推送
   精確標籤。workflow 會在建置前比對所有已抓取的標籤。
6. **發布流程會序列化並排隊。** 固定使用同一個 `release-image` concurrency group 並設定
   `queue: max`，最多保留 100 個 pending 發布；因此連續推送多個標籤時每個精確版本都會發布，
   較慢的舊版建置也不可能在較新版本之後完成並把浮動標籤往回移。
7. **不要對舊標籤勾選浮動標籤重建。** 那會把 `latest`（以及 `X`、`X.Y`）指回舊版本。舊標籤請
   使用預設值（只重建精確版本）；最新版檢查也會阻擋，但預設值讓意圖更明確。
8. **預發布版永遠不會有 `latest`。** 請使用 `-rc.1`、`-beta.1`、`-alpha.1` 之類的後綴。
9. **GitHub Release 由人工發布，格式固定。** 映像發布完成後，用
   [`scripts/release-notes.sh`](../../scripts/release-notes.sh) 產生草稿，並依
   [`.github/release-notes-template.md`](../../.github/release-notes-template.md) 撰寫。每一段
   散文與每個 highlight 項目都放在同一行——GitHub 會把 release body 內的換行渲染成強制換行
   ——並用 [`scripts/release-notes-lint.sh`](../../scripts/release-notes-lint.sh) 檢查草稿。
10. **GHCR 可見性是儲存庫設定。** 私有儲存庫會產生私有套件；請執行
    `docker login ghcr.io`（token 需要 `read:packages`），或把套件設為公開。
11. **[docker-compose.yml](../../docker-compose.yml) 的 `image:` 是本機建置標籤，但仍必須帶上發布版本。**
    它不會發布任何東西——正式映像一律來自 workflow——但漏改會讓 `docker compose up --build`
    把本機建置標成舊版本，`v1.3.0-rc.6` 就是這樣留下仍寫著 `v1.3.0-rc.5` 的 Compose 檔。
    請讓它與 `Cargo.toml`、`Cargo.lock` 保持一致。

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

這些檢查也會在 CI 執行：`.github/workflows/ci.yml` 會在每個 pull request，以及每個尚未被通過的
pull request 涵蓋的 `master` 推送，執行格式化、lint、測試、smoke 與 OpenAPI 合約檢查。建立標籤前
仍應在本機跑完上面這組指令，作為品質關卡，因為出錯的發布標籤無法重複使用。

### 2. 升版

版本號會出現在三個檔案，三處必須一起改：

| 檔案 | 要改的地方 |
|------|-----------|
| [`Cargo.toml`](../../Cargo.toml) | `version = "1.2.3"` |
| [`Cargo.lock`](../../Cargo.lock) | `urlet` 套件那一筆，由 `cargo update -p urlet` 更新 |
| [`docker-compose.yml`](../../docker-compose.yml) | `image: urlet:v1.2.3` |

```toml
# Cargo.toml
version = "1.2.3"
```

```yaml
# docker-compose.yml
image: urlet:v1.2.3
```

更新 lockfile 並確認它是最新的：

```bash
cargo update -p urlet
cargo metadata --locked --format-version 1 >/dev/null
```

內嵌的 API 參考也會帶上同一個版本（`info.version`），因此要一併重新產生合約並提交；提交的
文件過期時 CI 的 `contracts` job 會失敗：

```bash
python3 scripts/build-openapi.py     # 需要 PyYAML
```

提交版本變更與重新產生的合約：

```bash
git add Cargo.toml Cargo.lock docker-compose.yml openapi/dist
git commit -m "🔖 release: v1.2.3"
git push origin master
```

本專案使用 gitmoji 風格的主旨；`🔖 release: vX.Y.Z` 與現有歷史一致。
建立標籤前先確認三處一致：

```bash
grep -m1 '^version' Cargo.toml
grep -m1 -A2 '^name = "urlet"' Cargo.lock
grep -m1 'image: urlet:' docker-compose.yml
```

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
docker buildx imagetools inspect ghcr.io/cxyfer/urlet:v1.2.3
docker run --rm ghcr.io/cxyfer/urlet:v1.2.3 version   # 印出 "urlet 1.2.3"
```

`imagetools inspect` 必須同時列出 `linux/amd64` 與 `linux/arm64`。

### 5. 發布 GitHub Release

先產生草稿，再補上摘要與重點：

```bash
./scripts/release-notes.sh v1.2.3      # 產生 What's Changed、New Contributors、Full Changelog
$EDITOR .tmp/release-notes-v1.2.3.md   # 補上 callout 與 ## Highlights
./scripts/release-notes-lint.sh .tmp/release-notes-v1.2.3.md

gh release create v1.2.3 --title "urlet 1.2.3" \
  --notes-file .tmp/release-notes-v1.2.3.md --verify-tag
```

callout 的每一段與每個 highlight 項目都請寫在同一行；草稿若仍有換行，lint 會直接失敗。
GitHub 會把 release body 內的一個換行渲染成強制換行，因此原始檔的換行會讓發布頁面從句子
中間斷開。

`-rc.N` 標籤請加上 `--prerelease`。格式由
[`.github/release-notes-template.md`](../../.github/release-notes-template.md) 固定：開頭是
`> [!IMPORTANT]` 摘要與升級注意事項，接著是依領域分組的 `## Highlights`，然後是
`## What's Changed`、`## New Contributors`（有人首次貢獻時才出現）與 `**Full Changelog**`。
此時映像已經可以使用。

### 6. 選用：允許匿名拉取

若希望匿名 `docker compose pull` 可以運作，請到套件頁面設定
*Package settings → Danger zone → Change visibility → Public*。

## 手動重建

*Actions → Release container image → Run workflow*：

- `tag`：要重建的既有標籤（例如 `v1.2.3`）。
- `update_moving_tags`：除非你重建的是**最新正式版本**，且希望 `vX`、`vX.Y`、`latest` 跟著
  新 digest 移動，否則請保持**關閉**。最新版檢查會拒絕對較舊版本移動浮動標籤。

常見用途：修補基底映像的 CVE，或 runner/toolchain 變更，但不需要新的應用程式版本。

## 清理重建留下的版本

重建會以相同標籤產生新 digest，舊的 package version 會留在 registry。GHCR 只能刪 version、
不能刪單一 tag，所以刪掉舊 version 會一併移除它上面的所有標籤。確認 `v` 開頭的標籤與
`latest` 都已指向新 digest 後，執行 *Actions → Delete a container image version*，輸入已不再
使用的裸標籤（例如 `1.3.0-rc.1`）。workflow 會拒絕刪除 v 開頭的標籤，以及仍帶有 v 標籤或
`latest` 的 version；舊 version 的裸標籤會一起消失。

## 失敗處理

| 情況 | 處理方式 |
|------|----------|
| 版本不一致（validate job 失敗） | 刪除標籤（`git tag -d vX.Y.Z`、`git push origin :refs/tags/vX.Y.Z`），修正 `Cargo.toml`／`Cargo.lock` 並提交後重新建立標籤 |
| 建置失敗且尚未發布任何映像 | 在 `master` 修正，刪除標籤後重新建立 |
| 映像已發布但有問題 | 不要重用標籤。升 patch 版本重新發布；期間在部署端固定使用前一個版本 |
| 標籤在 workflow 存在前就已推送 | 用該標籤手動執行 workflow（關閉浮動標籤） |
| 無法拉取套件 | 執行 `docker login ghcr.io`，或把套件設為公開 |
| 需要前一個映像 | 設定 `URLET_IMAGE=ghcr.io/cxyfer/urlet:<舊版本>` 並重建容器 |

## 快速複製指令

```bash
VERSION=1.2.3

sed -i "s/^version = \".*\"$/version = \"$VERSION\"/" Cargo.toml
sed -i "s/^\(    image: urlet:v\).*$/\1$VERSION/" docker-compose.yml
cargo update -p urlet
cargo metadata --locked --format-version 1 >/dev/null
python3 scripts/build-openapi.py

grep -m1 '^version' Cargo.toml            # 1.2.3
grep -m1 -A2 '^name = "urlet"' Cargo.lock # 1.2.3
grep -m1 'image: urlet:' docker-compose.yml  # urlet:v1.2.3

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

git add Cargo.toml Cargo.lock docker-compose.yml
git commit -m "🔖 release: v$VERSION"
git push origin master
git tag -a "v$VERSION" -m "v$VERSION"
git push origin "v$VERSION"

# 映像發布完成後：
./scripts/release-notes.sh "v$VERSION"
$EDITOR ".tmp/release-notes-v$VERSION.md"
./scripts/release-notes-lint.sh ".tmp/release-notes-v$VERSION.md"
gh release create "v$VERSION" --title "urlet $VERSION" \
  --notes-file ".tmp/release-notes-v$VERSION.md" --verify-tag
```
