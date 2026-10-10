# Agent Session Bridge

本地管理与迁移 **Codex ↔ Claude Code** 开发会话的桌面工具，基于 Tauri 2 + React + TypeScript + Rust。[English](README.md)

## 功能

- 三栏工作台：工具/项目导航、会话列表、对话阅读器；支持正文搜索、日期筛选、排序和多选。
- 展示 Markdown、代码、可读推理、工具调用与结果；长工具输出折叠，长会话分批展示。
- 别名、标签、收藏、归档、移除和恢复仅修改 Bridge 自己的数据，保留源会话。
- **原生转换**创建新 ID，转换用户与助手消息及可准确对应的工具记录；Codex 使用官方 app-server 登记，不直接写其 SQLite。
- **上下文交接**本地确定性整理，可编辑、复制和保存 Markdown，并附完整归档位置；不调用模型、不消耗 API 额度。
- 导入 JSONL / `.asb` 前逐项预览；导出 Markdown、原始 JSONL、可重新导入的 `.asb` ZIP；批量迁移/导出展示进度，支持取消未执行项目和重试迁移失败项。
- 中英切换、系统/浅色/深色主题；可配置会话目录、CLI 和导出目录；没有 CLI 仍能浏览、管理和导出。
- 每 15 秒后台增量扫描，预览按需读取，也可手动刷新。

## 开发与打包

需要 Node.js 24、rustup（仓库固定 Rust 1.97.0）及 [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)：macOS 使用 Xcode 命令行工具；Windows 使用 C++ Build Tools 和 WebView2；Linux 安装 WebKitGTK 4.1 等开发库。

```sh
npm ci
npm run desktop          # 启动桌面开发版
npm run bundle           # 构建正式应用和安装包
```

构建产物位于 `src-tauri/target/release/bundle/`：macOS 为 `.app` / `.dmg`，Windows 为 MSI / NSIS，Linux 为 deb / AppImage。本地 Mac 包未签名、公证，如系统提示，请在 Finder 中右键选择“打开”。CI 配置会构建三平台并上传构建产物，不自动发布 Release。

```sh
npm run dev:demo         # 仅合成数据的浏览器演示
npm run build
npm test
npm run test:core
npx playwright install chromium
npm run test:gui
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --all-targets -- -D warnings
npm run test:integration # 需要已验证版本的两个 CLI；仅连接本地模拟模型
```

正式构建不要设置 `VITE_DEMO=1`。隔离测试可通过 `ASB_DATA_DIR` 指定 Bridge 数据目录。

## 使用流程

1. 在设置中确认会话目录和 CLI。默认读取 `~/.codex` / `~/.claude`，也识别 `CODEX_HOME` / `CLAUDE_CONFIG_DIR`。
2. 选择已有会话，或导入 `.jsonl` / `.asb`。导入先创建 Bridge 内部副本，再点击迁移选择目标。
3. 选择目标工具、已存在的项目目录及原生转换/上下文交接模式。检查降级报告或编辑交接文档。
4. 创建后复制恢复命令或打开终端继续。批量操作对所有选中会话使用同一个目标项目目录。

原生转换当前仅对 **Codex 0.160.1** 和 **Claude Code 2.1.280** 开放。在 macOS 使用隔离目录和 localhost 模拟模型验证过双向恢复，下一轮请求确实包含导入历史。其他版本可浏览、导出和上下文交接，不标记为原生兼容。详见[兼容性与验证记录](docs/compatibility.md)。

## 数据与恢复

Bridge 数据目录：

- macOS：`~/Library/Application Support/io.github.masamiyui.agent-session-bridge`
- Windows：`%APPDATA%/io.github.masamiyui.agent-session-bridge`
- Linux：`$XDG_DATA_HOME/io.github.masamiyui.agent-session-bridge`，通常是 `~/.local/share/...`

目录包含独立 SQLite、设置、导入副本、迁移日志及源归档。移除源会话的列表条目只改变 Bridge 元数据。需要保留收藏/标签/交接记录时，不要删除 Bridge 数据目录。

`.asb` v1 包含版本化 manifest、标准会话、SHA-256 校验的原始 JSONL、已收集附件和 Claude 子会话原始记录。缺失附件列入报告；导入的裸 JSONL 不会按历史路径读取本机任意附件。解包限制单文件 256 MB、展开总计 1 GB、10,000 项，拒绝路径穿越、软链接、重复/大小写冲突路径和 Windows 保留名称。

写入通过临时文件或目录提交，拒绝覆盖现有文件。重复迁移默认复用结果，也可明确创建副本。应用启动时恢复已保存结果或验证中断的原生目标；无法自动恢复时保留文件、记录原因，并阻止静默重复创建。阅读器不执行 HTML、脚本或历史命令。

归档可能包含私有提示、代码、文件路径和工具输出，分享前请检查内容。

## 首版边界

- `Bash` ↔ `exec_command` 等准确映射保留工具关联，无法映射或未配对的记录转为明确标记的历史文本。
- 加密推理、运行权限/状态、子 Agent 拓扑、图像上下文和压缩边界不伪装成可继续执行的状态；原始记录保留在归档。
- 交接文档包含全部用户文本块（每块最多 2,000 字符）及最近 24 条消息（每块最多 6,000 字符），保存时附完整归档路径。
- Claude 每次导入验证 ID 与 JSONL 结构；指定 CLI 版本另行通过实际恢复与继续测试。应用不会为了验证每次导入发起模型请求。
- 不提供云上传、跨设备同步和持续双向消息同步。打开目标 CLI 继续工作后，使用该 CLI 自身配置与计费。

源码按 GUI、解析、SQLite、归档、迁移及协议模块组织，保留 [Apache-2.0 许可证](LICENSE)。
