# unifuck Rust 工程评审报告

## 评审范围

本次评审面向 `unifuck` 仓库的 Rust 工程体系、模块边界、依赖治理、质量门禁、测试可维护性和仓库治理。评审不评价功能需求是否成立，不验证第三方授权或许可流程是否可用，也不提供绕过第三方校验的实现建议。

项目当前是一个 Rust + Slint 的 Windows 桌面工具。核心入口在 `src/main.rs`，UI 资源在 `ui/`，主要文件操作和许可证相关资源处理集中在 `src/core.rs`，Unity Hub `app.asar` 处理位于 `src/hub/`。

## 依据与项目事实

- 仓库根目录存在 `Cargo.toml`、`Cargo.lock`、`.gitignore`、`README.md`、`.editorconfig` 和 `.gitattributes`。
- 仓库没有 `rust-toolchain.toml`、`rustfmt.toml` 和 `clippy.toml`。
- `Cargo.toml` 使用 Rust 2021 edition，主要依赖包括 `slint`、`windows-sys`、`rsa`、`sha1`、`sha2`、`pem`、`roxmltree`、`serde_json`、`anyhow`。
- `Cargo.lock` 已提交，符合当前作为桌面应用发布的项目形态。
- `target/`、IDE 文件、日志和备份文件已被 `.gitignore` 忽略。

## 已执行检查

| 检查项 | 命令 | 结果 |
| --- | --- | --- |
| 格式检查 | `cargo fmt --all -- --check` | 通过 |
| 静态检查 | `cargo clippy --all-targets --all-features -- -D warnings` | 通过 |
| 测试 | `cargo test --all-targets --all-features` | 通过，19 个测试通过 |

以上结果只说明当前 Rust 格式、Clippy 和测试在本机检查通过，不代表第三方程序补丁行为、管理员权限流程、真实 Unity 安装环境、Unity Hub 版本兼容性或发布产物已完成端到端验证。

## 总体结论

项目已经具备基础的 Rust 工程门禁：格式检查、Clippy 和测试在本地检查通过。代码按 GUI、Hub、ASAR、版本策略和核心文件操作做了初步拆分，整体不是无结构堆叠。

主要工程短板集中在两个方向：核心文件操作边界过宽，内置补丁素材命名容易被误解。仓库说明、文本治理配置和默认 ASAR 测试的可复现性已经补齐，后续重点应放在模块边界收敛。

## 主要问题

### 核心模块职责过宽

**级别：中**

`src/core.rs` 同时负责嵌入资源、Unity 安装扫描、二进制证书替换、许可证签名和部署、hosts 写入、备份还原、共享 Licensing Client 处理等职责。该文件已经接近平台文件操作的总入口，后续增加 Unity 版本适配、回滚策略或错误恢复时，容易让核心流程继续膨胀。

建议将职责拆为更明确的模块：

- `patch_assets`：集中管理内置补丁素材、证书、许可证模板和命名说明。
- `install_scan`：负责 Unity Editor 和 Hub 相关路径发现。
- `binary_patch`：负责二进制内容匹配、替换、备份和还原。
- `license_file`：负责许可证模板签名和部署。
- `hosts_file`：负责 hosts 标记段写入和清理。

拆分重点不是改变行为，而是把高风险文件写入、备份、还原和内置素材边界收窄，降低后续维护成本。

### 内置补丁素材命名容易造成误解

**级别：中**

仓库提交了 `keys/mod/priv.key`、`keys/mod/*.cer`、`keys/orig/*.cer` 和 `license.xml`。结合当前用途，它们更像用于替换和生成输出的内置补丁素材，不一定是需要保密的生产密钥。但目录名和文件名使用 `keys`、`priv.key`，会让安全扫描、代码审查和后续维护者自然按敏感密钥理解。

建议保留仓库内置方式，但调整工程语义：

- 将目录改为 `patch_assets/`、`embedded_payloads/` 或同类名称。
- 用 `README` 或模块注释说明这些文件是内置补丁素材，不是生产凭据。
- 通过一个 Rust 模块统一导出 `MOD_CERT_PEM`、`ORIG_CERT_PEM`、`LICENSE_TEMPLATE_XML` 等常量，避免 `src/core.rs` 直接散落资源路径。

这项调整不改变发布模型，也不要求运行时外置配置，主要解决误判和边界表达问题。

## 观察项

### 平台边界集中在 Windows

代码中存在 Windows 路径、注册表读取、`windows-sys`、Windows manifest 和管理员权限要求。虽然部分代码通过 `cfg(windows)` 做了条件编译，但项目事实是 Windows 桌面工具。建议 README 和发布矩阵明确以 Windows 为主，避免调用方误判跨平台支持状态。

### 备份和还原策略需要文档化

项目使用 `.bak` 文件判断 patch 状态，并通过备份文件还原。该策略简单直接，但对异常中断、部分写入成功、备份已存在、原文件更新后旧备份残留等场景较敏感。当前代码已有一定保护，例如部分路径会在未替换成功时删除备份，但整体策略还缺少文档化说明。

建议在后续重构时为备份状态定义明确规则：哪些 `.bak` 代表已 patch，哪些失败会回滚，哪些文件允许静默跳过，哪些错误必须中止。

### ASAR 写回实现需要 fixture 覆盖更多格式

`src/hub/asar.rs` 已实现 ASAR 读取、写回和 integrity 计算。默认测试已改为自包含 fixture，并覆盖嵌套目录、多文件 offset、header 对齐和修改文件读回。后续可以继续增加空文件、异常 header 和不存在 entry 等边界样本。

## 建议推进顺序

1. 将 `keys/` 和 `license.xml` 归入内置补丁素材模块，解决命名误解和资源路径散落问题。
2. 将 `src/core.rs` 按职责拆分，保持现有行为不变，先用当前测试和 Clippy 兜住回归。
3. 继续补充 ASAR 边界样本，覆盖空文件、异常 header 和不存在 entry 等路径。

## 交付判断

当前项目具备继续迭代的基础工程质量，格式、Clippy 和测试在本机通过。若要作为可长期维护的个人桌面工具继续推进，建议优先治理模块边界、测试可复现性和必要的仓库说明，不需要引入超出项目规模的协作流程。补丁行为本身涉及第三方程序文件和系统文件写入，应持续把权限、备份、回滚和合规边界作为工程设计的一部分管理。
