# 運動拍檔上傳工具 RaceShot Photo Uploader

專為運動賽事攝影師設計的桌面上傳工具，支援將大量賽事照片快速上傳至 RaceShot 平台，並自動匹配選手資料。

![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)
![Version](https://img.shields.io/github/v/release/raceshot/uploader-v2)
![License](https://img.shields.io/badge/license-proprietary-red)

---

## 下載安裝

前往 [Releases](https://github.com/raceshot/uploader-v2/releases/latest) 下載最新版本：

| 平台 | 檔案 |
|------|------|
| macOS (Apple Silicon / Intel) | `.dmg` |
| Windows | `.exe` |

### macOS 安裝注意

首次開啟時 macOS 可能顯示「無法驗證開發者」警告，請：

1. 前往「**系統設定 → 隱私權與安全性**」
2. 找到「已封鎖使用運動拍檔上傳工具」
3. 點擊「**仍要開啟**」

---

## 功能

- **網頁登入**：透過 RaceShot 帳號授權，安全無需輸入密碼
- **活動選擇**：登入後自動載入可上傳的賽事活動
- **資料夾掃描**：選擇照片資料夾，自動掃描所有 JPG/JPEG
- **拍攝地點**：設定地點名稱與 GPS 座標，支援地圖選點
- **GPX 匹配**：匯入 GPX 軌跡檔，依拍攝時間自動匹配座標
- **批次上傳**：多張照片同時上傳，顯示即時進度與日誌
- **上傳歷史**：記錄已上傳檔案，避免重複上傳
- **自動更新**：啟動時自動偵測新版本並提示更新

---

## 使用流程

1. **登入**：點擊「網頁登入」，完成授權後自動返回
2. **選擇活動**：點擊「更新活動」載入活動列表，選擇目標活動
3. **選擇資料夾**：點擊「選擇資料夾」指定照片所在位置
4. **設定地點**（選填）：輸入拍攝地點名稱，或使用「地圖選點」取得 GPS 座標
5. **GPX 匹配**（選填）：選擇 `.gpx` 檔案，系統會依拍攝時間自動填入座標
6. **開始上傳**：點擊「開始上傳」，即時查看上傳進度與結果

---

## 系統需求

| | 最低需求 |
|---|---|
| macOS | 10.13 High Sierra 以上 |
| Windows | Windows 10 以上 |

---

## 開發

本專案使用 [Tauri 2](https://tauri.app) + [Vue 3](https://vuejs.org) + Rust 開發。

```bash
# 安裝依賴
pnpm install

# 啟動開發模式
pnpm tauri dev

# 打包
pnpm tauri build
```

Release 打包透過 GitHub Actions 自動化，push `v*` tag 即可觸發。

---

© 運動拍檔 RaceShot｜[raceshot.app](https://raceshot.app)
