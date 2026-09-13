# 本地打包说明

## 前置

- Rust 工具链（`rustup`），MSVC 工具链推荐：`stable-x86_64-pc-windows-msvc`
- Node.js 18+
- 首次构建会自动下载 Tauri 依赖，需要网络

## 1. 编译

```powershell
npm ci
npx tauri build
```

产物在 `src-tauri/target/release/`：

| 文件 | 说明 |
|---|---|
| `dbclient.exe` | 主程序 |
| `WebView2Loader.dll` | 运行时依赖（MSVC 工具链下可能不存在，embedBootstrapper 已内嵌引导器） |
| `bundle/nsis/dbclient_<版本>_x64-setup.exe` | NSIS 安装包 |
| `bundle/msi/dbclient_<版本>_x64_en-US.msi` | MSI 安装包 |

> 版本号读自 `src-tauri/tauri.conf.json` 的 `version` 字段。

## 2. 绿色版（portable）+ 桌面快捷方式

一键脚本：

```powershell
powershell -ExecutionPolicy Bypass -File build-portable.ps1
```

脚本做了三件事：

1. 把 `dbclient.exe`（和 `WebView2Loader.dll`，如果存在）复制到 `dist-portable/dbclient-<版本>-win-x64/`
2. 打包成 `dist-portable/dbclient_<版本>_portable_win-x64.zip`
3. 在桌面创建 `dbclient.lnk` 快捷方式，指向绿色版 exe（工作目录已设好）

> 前提：第 1 步已编译成功。`dist-portable/` 已被 `.gitignore` 排除。

## 3. 发布到 GitHub Release（可选）

```powershell
git tag v<版本号>
git push origin main v<版本号>
```

推送 `v*` tag 后，GitHub Actions 会自动在 Windows 上构建 NSIS / MSI / 绿色版 zip 三个文件，并创建对应的 GitHub Release。

本地手动上传（不走 CI）：

```powershell
gh release create v<版本号> \
  src-tauri/target/release/bundle/nsis/dbclient_<版本号>_x64-setup.exe \
  src-tauri/target/release/bundle/msi/dbclient_<版本号>_x64_en-US.msi \
  dist-portable/dbclient_<版本号>_portable_win-x64.zip
```

## 常用命令速查

| 任务 | 命令 |
|---|---|
| 开发模式 | `npm run tauri dev` |
| 全量构建 | `npx tauri build` |
| 绿色版 + 快捷方式 | `powershell -ExecutionPolicy Bypass -File build-portable.ps1` |
| 只重打绿色版 zip | 同上（复用已有 exe） |
| 本地上传 release | `gh release create v<版本> <文件>` |
