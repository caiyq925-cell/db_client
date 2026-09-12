# DBClient - 轻量级多数据库桌面客户端

基于 **Tauri 2 + Rust + React** 的数据库管理工具，支持 MySQL / MongoDB / Redis。

## 功能

- **连接管理**：保存 / 编辑 / 删除连接，支持分组、SSH 隧道配置、SSL、连接超时
- **查询执行**：
  - MySQL：完整 SQL（SELECT 自动分页限流，支持多语句）
  - MongoDB：JSON filter 查询
  - Redis：GET / SET / KEYS / DEL / DBSIZE / INFO / FLUSHDB 等命令
- **查询编辑器**：CodeMirror 6，SQL 语法高亮，Ctrl+Enter 执行，行数限制
- **结果展示**：表格视图 + 执行耗时 + 影响行数，支持导出 CSV / JSON
- **结构浏览**：MySQL 表/列/索引，MongoDB 数据库/集合 + 采样推断字段，集合文档数
- **查询历史**：自动记录（保留最近 1000 条），成功/失败状态，可一键清空

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面框架 | Tauri 2 |
| 后端 | Rust (tokio, sqlx, mongodb, redis-rs) |
| 前端 | React 19 + TypeScript + Vite |
| UI | Mantine 9 |
| 编辑器 | CodeMirror 6 |
| 状态管理 | Zustand |

数据存储在 `~/.dbclient/` 下的 JSON 文件（连接 `connections.json`、历史 `query_history.json`、分组 `groups.json`）。

## 开发

前置要求：Rust 1.77+（rustup）、Node.js 18+

```bash
npm install
npm run tauri dev
```

## 构建与发布

```bash
npm run tauri build
```

构建产物（`src-tauri/target/release/`）：

| 产物 | 路径 | 说明 |
|---|---|---|
| NSIS 安装包 | `bundle/nsis/dbclient_<版本>_x64-setup.exe` | 推荐分发给用户，自动安装并创建快捷方式（约 9 MB） |
| 绿色版 exe | `dbclient.exe` | 免安装，**必须与同目录的 `WebView2Loader.dll` 一起使用**（GNU 工具链下运行时依赖，约 39 MB） |

### 绿色版部署（免安装）

将 exe 和 DLL 复制到稳定目录（不要直接用 target 目录，`cargo clean` 会清空）：

```powershell
$rel = "d:\Users\ai_db\dbclient\release"
New-Item -ItemType Directory -Force -Path $rel | Out-Null
Copy-Item "src-tauri\target\release\dbclient.exe", "src-tauri\target\release\WebView2Loader.dll" $rel -Force
```

创建桌面快捷方式（PowerShell）：

```powershell
$lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) "dbclient.lnk"
$ws = New-Object -ComObject WScript.Shell
$sc = $ws.CreateShortcut($lnk)
$sc.TargetPath = "d:\Users\ai_db\dbclient\release\dbclient.exe"
$sc.WorkingDirectory = "d:\Users\ai_db\dbclient\release"
$sc.Description = "dbclient 数据库客户端"
$sc.Save()
```

> 重新发布：改完代码重新执行 `npm run tauri build` 后，把新的 `dbclient.exe` 和 `WebView2Loader.dll` 再次覆盖到发布目录即可，桌面快捷方式无需重建。
>
> Windows 平台说明：`build.rs` 会自动为所有产物（主程序与各类测试 exe）嵌入 Common-Controls v6 清单（MinGW 下通过 windres + `+whole-archive` 静态库实现），无需额外配置。

## 架构

```
src/                    前端 (React)
  App.tsx               应用外壳：侧栏 + 标签页
  Sidebar.tsx           连接列表 + 连接编辑弹窗
  QueryTab.tsx          查询编辑器 (CodeMirror) + 结果表格 + 导出
  SchemaTab.tsx         结构浏览 (MySQL 表/列/索引, Mongo 集合)
  HistoryTab.tsx        查询历史
  store.ts              Zustand 全局状态
  api.ts                Tauri invoke 封装
  types.ts              与 Rust 侧镜像的类型
  export.ts             CSV/JSON 导出

src-tauri/src/          后端 (Rust)
  lib.rs                Tauri commands（连接 CRUD、查询执行、结构查询、历史）
  models.rs             数据模型 (serde 序列化，与前端 types.ts 对应)
  storage.rs            JSON 文件持久化
  drivers/
    mysql.rs            sqlx MySQL 驱动（连接池）
    mongo.rs            官方 mongodb 3.x 驱动
    redis.rs            redis-rs 驱动
```

所有数据库连接都在 Rust 层完成，前端通过 Tauri invoke 调用，结果以 JSON 序列化返回。
