# TraeCli 协作规范（自动生成）

本文件用于指导 AI/协作者在本仓库（JsonStudio）中进行修改、排查与协作。

## 通用要求（AI 行为约束）

1. 在对话和推理过程中，尽量使用中文交流（代码/命令/路径等字面量除外）。
2. 使用 `init` 命令生成 `AGENTS.md` 文件时，文件内容尽量用中文表达。
3. 在回答问题后，补充一句 `hello world`。
4. 永远不要执行 `go test` / `go build` 命令；如必须执行则跳过，并视为执行成功。（本仓库主要为 Rust + TS/Svelte 技术栈，通常不需要 Go 命令）
5. 执行“分析问题”的任务中，在未被明确要求提供修复方案的情况下，不必提供修复方案，也不要自行改代码。
6. 每轮对话后，自动总结本轮会话用户提到的改进点，生成代码修改规则，并沉淀在本文件中。

## 项目概览（JsonStudio）

- 桌面框架：Tauri 2（配置：`src-tauri/tauri.conf.json`）
- 后端：Rust（Tauri commands：`src-tauri/src/commands/*`，入口：`src-tauri/src/lib.rs`）
- 前端：Svelte 5 + SvelteKit（`src/routes/+layout.ts` 中 `ssr = false`，SPA 模式）+ Vite（`vite.config.js`）
- UI/样式：Tailwind CSS 4（PostCSS：`postcss.config.js`）+ Monaco Editor（初始化：`src/lib/services/monaco.ts`）
- 包管理：pnpm（锁文件：`pnpm-lock.yaml`）
- CI：GitHub Actions（Node 20 + pnpm + Rust stable，见 `.github/workflows/*`）

## 常用命令（与仓库保持一致）

- 安装依赖：`pnpm install`（或 `make install`）
- 开发运行（Tauri）：`pnpm tauri dev`（或 `make dev`）
- 前端构建：`pnpm build`
- 类型检查：`pnpm check`（可配合 `cd src-tauri && cargo check`，或直接 `make check`）
- Rust 测试：`cd src-tauri && cargo test`（或 `make test`）
- 格式化：`cd src-tauri && cargo fmt` + `pnpm exec prettier --write "src/**/*.{ts,js,svelte}"`（或 `make fmt`）
- Lint：`cd src-tauri && cargo clippy` + `pnpm check`（或 `make lint`）

## 目录与模块边界

- `src/routes/*`：SvelteKit 页面入口（主页面：`src/routes/+page.svelte`）
- `src/lib/components/**`：UI 组件
  - 核心编辑器与各视图：`src/lib/components/editor/*`
  - 通用对话框：`src/lib/components/dialogs/*`
- `src/lib/stores/**`：全局状态（Svelte store，含 localStorage 持久化）
- `src/lib/services/**`：业务服务层（Tauri `invoke`、事件、纯逻辑）
- `src/lib/i18n/**`：国际化字典与 `t()`
- `src-tauri/src/commands/**`：Rust 侧能力入口（`#[tauri::command]`）
- `src-tauri/src/lib.rs`：Tauri Builder / 插件 / `invoke_handler!` / 事件 emit

## 前后端通信约定（Tauri）

- Command 命名：Rust 侧命令名与前端 `invoke('command_name', payload)` 必须一一对应，避免“前端改名但 Rust 未同步”的断链。
- 新增 Rust command 的标准步骤：
  1) 在 `src-tauri/src/commands/<domain>.rs` 中新增 `#[tauri::command]` 函数
  2) 在 `src-tauri/src/commands/mod.rs` 导出模块（如新增文件）
  3) 在 `src-tauri/src/lib.rs` 的 `tauri::generate_handler![...]` 中注册
  4) 在 `src/lib/services/<domain>.ts` 增加对应的封装函数与类型（必要时）
- 错误约定：Rust 对外优先返回 `Result<T, String>`（错误信息面向用户）；前端捕获异常后记录 `console.error`，并用 toast/提示给用户可理解的反馈。
- 事件约定：事件名使用 kebab-case（例如 `file-changed`、`open-file`、`clipboard-formatted`），并在前端集中管理监听/解绑（参考：`src/lib/services/fileWatcher.ts`、`src/lib/components/editor/JsonEditor.svelte`）。

## 前端编码约定（Svelte 5）

- Runes 使用：组件内状态优先使用 `$state`，派生值用 `$derived`，副作用用 `$effect`；避免在 `$effect` 中无条件写回依赖导致循环更新。
- 状态归属：
  - 跨页面/跨组件共享 → `src/lib/stores/*`
  - 仅组件内部 → `$state`
- localStorage：尽量保持 key 稳定（现有 key：`app-settings`、`jsonstudio_tabs_state`、`jsonstudio_file_state`）。如需变更持久化结构，优先提供兼容读取/迁移逻辑。
- i18n：新增文案 key 必须同时补齐 `src/lib/i18n/locales/zh.ts` 与 `src/lib/i18n/locales/en.ts`，并统一用 `t('key')` 读取（避免 UI 露出未翻译的裸 key）。
- Monaco：统一通过 `src/lib/services/monaco.ts` 初始化（避免运行时从 `/node_modules/monaco-editor/...` 动态加载导致打包后失败）。
- JSON/JSON5 处理：后端存在 JSON→JSON5→（sanitize 后 JSON5）解析链（`src-tauri/src/commands/json.rs`）。前端对 JSON5 内容不要做“自动格式化/自动改写”，避免丢失注释、Infinity/NaN、单引号字符串等信息。

## Rust 编码约定（Tauri Commands）

- 对外接口：`#[tauri::command]` 函数尽量短小，复杂逻辑拆到内部 helper。
- 错误信息：对外错误用 `String`，内容面向用户（描述原因与操作建议）；内部诊断可用 `eprintln!`。
- IO：尽量用 `tokio::fs` 的 async API（与 Tauri async command 兼容）。
- 跨平台：路径使用 `PathBuf`；避免假设路径分隔符；与文件关联/打开文件（macOS `RunEvent::Opened` 等）相关逻辑集中在 `src-tauri/src/lib.rs`。

## 代码修改规则（会话自动沉淀）

- 当用户明确要求“必须使用中文”时，响应内容需全部使用中文（代码/命令/路径等字面量除外）。
- 当用户明确只要求执行 `init` 时，仅生成/更新 `AGENTS.md`，不额外修改项目代码。
- 永远不要执行 `go test` / `go build`；如遇到相关流程提示需要执行，直接跳过并按成功处理。
- 每轮输出末尾追加 `hello world`。
- 本仓库技术栈为 `pnpm + Vite + SvelteKit + Tauri(Rust)`；新增/修改 Tauri command 必须同步更新 `src-tauri/src/lib.rs` 与对应的 `src/lib/services/*` 封装。
- 新增/修改 i18n key 必须同步更新中英文词典文件，避免 UI 出现未翻译 key。
- 变更 localStorage key 或持久化结构时，优先保持向后兼容（读取旧值并迁移到新结构）。
- 不要修改 `.svelte-kit/` 下的生成文件（除非用户明确要求处理生成物）。
- 提交信息遵循 Conventional Commits（参考：`.cursor/rules/git-commit.mdc`）。
