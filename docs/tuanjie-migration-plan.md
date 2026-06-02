# Tuanjie 补丁迁移方案

## 背景

unifuck 当前支持 Unity Hub (CommonJS/ESM 格式) 和 Unity Editor (2021 单体 exe + DLL 字节替换)。Tuanjie (团结引擎) 是 Unity 中国分支，目录结构和代码逻辑高度相似但存在关键差异。本文档分析迁移可行性并规划实施路径。

## 目标环境

| 组件 | 路径 | 版本 |
|------|------|------|
| Tuanjie Hub | `C:\Program Files\Tuanjie Hub` | 1.4.2 |
| Tuanjie Editor | `C:\Program Files\Tuanjie 2022.3.62t8\Editor` | 2022.3.62t8 |
| Shared Licensing Client | `C:\Program Files\Tuanjie Hub\TuanjieLicensingClient_V1` | - |

## Tuanjie Hub 迁移分析

### 现状对比

| 对比项 | Unity Hub | Tuanjie Hub | 兼容性 |
|--------|-----------|-------------|--------|
| ASAR 文件 | `resources\app.asar` | `resources\app.asar` | ✅ 一致 |
| JS 格式 | CommonJS / ESM | CommonJS | ✅ 支持 |
| `licenseService.js` | ✅ 存在 | ✅ 存在 | ✅ 一致 |
| `licenseServiceCore.js` | ✅ 存在 | ❌ 不存在 | ⚠️ 需适配 |
| `licensingSdk.js` | ✅ 存在 | ✅ 存在 | ✅ 一致 |
| `AuthService.js` | ✅ 存在 | ✅ 存在 | ✅ 一致 |
| 异步模式 | `__awaiter` / `async` | `__awaiter` | ✅ 一致 |

### JS 函数签名差异

#### licenseService.js

**`isLicenseValid()`**
- Unity Hub: 直接调用 `licensingSdk.getInstance().checkEntitlement()`
- Tuanjie Hub: 先检查 `localSettings` 配置项，再调用 SDK
- **迁移影响**: 补丁逻辑可直接复用，函数体替换目标一致

**`getValidEntitlementGroups()`**
- Unity Hub: 定义在 `licenseServiceCore.js`
- Tuanjie Hub: 定义在 `licensingSdk.js`，`licenseService.js` 仅调用
- **迁移影响**: 需要调整补丁目标文件，从 `licenseServiceCore.js` 改为 `licensingSdk.js`

#### licensingSdk.js

**`init(externalCorrelationId)`**
- Unity Hub: ✅ 存在
- Tuanjie Hub: ✅ 存在，签名一致
- **迁移影响**: 可直接复用补丁

**`getInstance()`**
- Unity Hub: ✅ 存在
- Tuanjie Hub: ✅ 存在
- **迁移影响**: 可直接复用补丁

#### AuthService.js

**`constructor()`**
- Unity Hub: 初始化 `tokenRefreshTimer`、`userInfoRefreshTimer`
- Tuanjie Hub: 初始化 `tokenWatcher`，字段名不同
- **迁移影响**: 需调整注入的 userInfo 结构，移除不存在的 timer 字段

**`isUserSignedIn()`**
- Unity Hub: ✅ 存在
- Tuanjie Hub: ✅ 存在
- **迁移影响**: 可直接复用补丁

**`restoreAuthState()`**
- Unity Hub: ✅ 存在，负责设置登录状态并广播事件
- Tuanjie Hub: ❌ 不存在，改用 `setLoggedInFlags()` + `init()` 流程
- **迁移影响**: 需重新实现登录状态注入逻辑

### 补丁策略调整

**新增 Tuanjie Hub 策略 (`tuanjie.rs`)**

```rust
pub(crate) struct TuanjieStrategy;

impl PatchStrategy for TuanjieStrategy {
    fn version_name(&self) -> &'static str {
        "Tuanjie Hub (CommonJS)"
    }

    fn applies_to(&self, asar: &AsarArchive) -> bool {
        // 检查 package.json 中 name 字段是否为 "TuanjieHub"
        // 或检查是否存在 tuanjieVersionMapping.js
    }

    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        // 1. licenseService.js: isLicenseValid() → return true
        // 2. licensingSdk.js: getValidEntitlementGroups() → return fake education license
        // 3. licensingSdk.js: init() → return true
        // 4. licensingSdk.js: getInstance() → return null
        // 5. AuthService.js: isUserSignedIn() → return true
        // 6. AuthService.js: constructor() → inject fake userInfo
        // 7. AuthService.js: setLoggedInFlags() → 设置 loggedIn/ready 状态
    }
}
```

**关键补丁内容**

```javascript
// AuthService.js constructor 注入
constructor() {
    super();
    this.userInfo = undefined;
    this.codeVerifier = '';
    this.authLogger = (0, logger_1.default)('Authentification Service');
    this.connectInfo = {
        error: false,
        initialized: false,
        loggedIn: false,
        maintenance: false,
        online: true,
        ready: false,
        showLoginWindow: false,
        workOffline: false,
    };
    this.tokenWatcher = null;
    // 注入 fake userInfo
    this.userInfo = {
        foreignKey: "unifuck-fk",
        name: "Unifuck",
        email: "bypass@local",
        primaryOrg: "local",
        createdAt: "2024-01-01T00:00:00.000Z",
    };
}

// AuthService.js setLoggedInFlags 修改
setLoggedInFlags() {
    this.connectInfo.ready = true;
    this.connectInfo.loggedIn = true;
    this.connectInfo.initialized = true;
    this.connectInfo.workOffline = false;
    this.userOrgs = null;
    windowManager_1.default.broadcastContent(PostalTopic_1.default.CONNECT_INFO_CHANGED, this.connectInfo);
    windowManager_1.default.broadcastContent(PostalTopic_1.default.USER_INFO_CHANGED, this.getFormattedUserInfo());
    windowManager_1.default.broadcastContent(RendererEvents_1.default.USER_SIGN_IN, this.userInfo);
}
```

### Hub 检测逻辑扩展

**`hub::find_hub()` 扩展**

```rust
// src/hub/mod.rs
pub(crate) fn find_hub() -> Option<HubInfo> {
    let candidates = vec![
        PathBuf::from(r"C:\Program Files\Unity Hub"),
        PathBuf::from(r"D:\Program Files\Unity Hub"),
        PathBuf::from(r"C:\Program Files\Tuanjie Hub"),  // 新增
        PathBuf::from(r"D:\Program Files\Tuanjie Hub"),  // 新增
    ];
    // ...
}
```

**注册表查找扩展**

```rust
// src/hub/mod.rs
#[cfg(windows)]
fn hub_path_from_registry() -> Option<PathBuf> {
    // 尝试 Unity Hub
    let unity_key = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Unity Hub";
    if let Some(path) = query_registry(unity_key) {
        return Some(path);
    }
    
    // 尝试 Tuanjie Hub
    let tuanjie_key = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Tuanjie Hub";
    query_registry(tuanjie_key)
}
```

## Tuanjie Editor 迁移分析

### 现状对比

| 对比项 | Unity Editor | Tuanjie Editor | 兼容性 |
|--------|-------------|----------------|--------|
| 主程序 | `Unity.exe` | `Tuanjie.exe` | ✅ 命名规则一致 |
| DLL 名 | `Unity.Licensing.EntitlementResolver.dll` | `Tuanjie.Licensing.EntitlementResolver.dll` | ⚠️ 前缀不同 |
| DLL 路径 | `Editor\Data\Resources\Licensing\Client\` | 同 | ✅ 一致 |
| 证书存储 | base64 文本嵌入 .NET IL | .NET 嵌入资源 (GetManifestResourceStream) | ❌ **机制不同** |
| 证书 CA | GlobalSign | DigiCert | ⚠️ CA 不同 |
| DLL 体积 | ~211 KB | ~211 KB | ✅ 一致 |

### 证书嵌入机制差异

**Unity Editor (2021)**
```csharp
// IL 代码中直接包含 base64 字符串
private const string CertBase64 = 
    "MIIE7zCCA9egAwIBAgIRAJZpZsDepakv5CaXJit+yx4wDQYJKoZIhvcNAQEFBQAw\n" +
    "VDELMAkGA1UEBhMCQkUxGTAXBgNVBAoTEEdsb2JhbFNpZ24gbnYtc2ExKjAoBgNV\n" +
    // ...
```

**Tuanjie Editor**
```csharp
// 从 .NET 嵌入资源加载
private X509Certificate2 LoadEmbeddedCertificate() {
    var assembly = Assembly.GetExecutingAssembly();
    using (var stream = assembly.GetManifestResourceStream(
        "Tuanjie.Licensing.EntitlementResolver.cert.cer")) {
        return new X509Certificate2(stream);
    }
}
```

### 当前方法不可行原因

1. **字节替换失效**: Tuanjie DLL 中证书不以 base64 文本形式存在于 IL，而是作为二进制 blob 存储在 PE 资源区
2. **PE 结构破坏**: 直接替换资源区字节会破坏 PE 文件的偏移表、RVA 引用和长度字段
3. **.NET 元数据引用**: 资源通过 `ManifestResource` 表引用，包含名称、偏移和长度，修改需同步更新元数据

### 迁移方案

**方案 A: .NET 资源替换 (推荐)**

使用 Mono.Cecil 或 dnlib 等 .NET 程序集操作库，在资源层面替换证书：

```csharp
using Mono.Cecil;

public static void PatchTuanjieDll(string dllPath, string newCertPath) {
    var assembly = AssemblyDefinition.ReadAssembly(dllPath);
    var module = assembly.MainModule;
    
    // 查找目标资源
    foreach (var resource in module.Resources) {
        if (resource.Name.Contains("EntitlementResolver") && 
            resource.ResourceType == ResourceType.Embedded) {
            var embedded = (EmbeddedResource)resource;
            
            // 读取新证书
            var newCertBytes = File.ReadAllBytes(newCertPath);
            
            // 替换资源内容
            module.Resources.Remove(resource);
            module.Resources.Add(new EmbeddedResource(
                resource.Name, 
                resource.Attributes, 
                newCertBytes));
            
            break;
        }
    }
    
    // 保存修改后的程序集
    assembly.Write(dllPath + ".patched");
}
```

**优点**:
- 保持 PE 结构完整性
- 自动处理偏移、长度、对齐
- 支持强名称签名重新计算

**缺点**:
- 需要引入 .NET 程序集操作依赖 (Mono.Cecil / dnlib)
- Rust 项目需通过 FFI 调用 .NET 工具，或改用 C# 编写补丁模块

**方案 B: IL 级修改 (复杂)**

反编译 DLL，修改 IL 代码将证书改为硬编码字符串，重新编译：

```bash
# 使用 ilspycmd 反编译
ilspycmd Tuanjie.Licensing.EntitlementResolver.dll -o decompiled

# 修改代码，将 LoadEmbeddedCertificate 改为返回硬编码证书
# 重新编译
csc /target:library /out:patched.dll decompiled/*.cs
```

**缺点**:
- 反编译可能丢失部分元数据
- 重新编译需要完整的依赖图
- 强名称签名需要重新生成密钥对

**方案 C: 运行时 Hook (不推荐)**

通过 DLL 注入或 AppDomain Hook 在运行时拦截证书加载：

```csharp
// 示例: Harmony Hook
[HarmonyPatch(typeof(EntitlementResolver), "LoadEmbeddedCertificate")]
class Patch {
    static bool Prefix(ref X509Certificate2 __result) {
        __result = new X509Certificate2(File.ReadAllBytes("mod.cer"));
        return false; // 跳过原方法
    }
}
```

**缺点**:
- 需要每次启动时注入
- 容易被反作弊系统检测
- 不适用于离线补丁场景

### 推荐实施路径

**阶段 1: 提取 Tuanjie 证书**

```python
# 使用 dnfile 或 PE-bear 提取 .NET 资源
import dnfile

pe = dnfile.dnPE("Tuanjie.Licensing.EntitlementResolver.dll")
for resource in pe.net.resources:
    if "EntitlementResolver" in resource.name:
        with open("tuanjie_cert.der", "wb") as f:
            f.write(resource.data)
```

**阶段 2: 生成修改版证书**

复用现有 `keys/mod/` 下的证书生成流程，但需适配 DigiCert CA：

```bash
# 生成新私钥
openssl genrsa -out tuanjie_mod.key 2048

# 生成自签名证书 (或请求 DigiCert 签发)
openssl req -new -x509 -key tuanjie_mod.key -out tuanjie_mod.cer -days 3650

# 转换为 DER 格式
openssl x509 -in tuanjie_mod.cer -outform DER -out tuanjie_mod.der
```

**阶段 3: 实现 .NET 资源替换工具**

选项 A: Rust + FFI
```rust
// 通过 cxx 桥接调用 Mono.Cecil
#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("unifuck/dotnet_patcher.h");
        unsafe fn patch_dotnet_resource(
            dll_path: &str,
            resource_name: &str,
            new_data: &[u8],
            output_path: &str,
        ) -> Result<()>;
    }
}
```

选项 B: 独立 C# 工具
```csharp
// TuanjiePatcher.exe
class Program {
    static int Main(string[] args) {
        var dllPath = args[0];
        var certPath = args[1];
        // 使用 Mono.Cecil 替换资源
        // ...
        return 0;
    }
}
```

Rust 侧通过 `std::process::Command` 调用：
```rust
pub(crate) fn patch_tuanjie_dll(dll_path: &Path) -> anyhow::Result<()> {
    let cert_data = include_bytes!("../keys/tuanjie/mod.der");
    let temp_cert = std::env::temp_dir().join("tuanjie_mod.der");
    std::fs::write(&temp_cert, cert_data)?;
    
    let status = std::process::Command::new("TuanjiePatcher.exe")
        .arg(dll_path)
        .arg(&temp_cert)
        .status()?;
    
    if !status.success() {
        anyhow::bail!("TuanjiePatcher failed");
    }
    
    Ok(())
}
```

**阶段 4: 集成到现有流程**

```rust
// src/core/binary_patch.rs 新增
pub fn patch_tuanjie_dll(dll_path: &Path) -> anyhow::Result<()> {
    // 检测是否为 Tuanjie DLL
    let is_tuanjie = dll_path.file_name()
        .map(|n| n.to_string_lossy().starts_with("Tuanjie."))
        .unwrap_or(false);
    
    if is_tuanjie {
        patch_tuanjie_dotnet_resource(dll_path)
    } else {
        patch_dll(dll_path) // 原有逻辑
    }
}
```

```rust
// src/gui/jobs.rs 修改 run_patch 函数
fn run_patch(tx: mpsc::Sender<String>, targets: JobTargets) {
    // ... existing code ...
    
    for inst in &targets.selected {
        if core::is_patched(&inst.dll_path) {
            tx.send(format!("TOK  {}  already patched, skipped", inst.version))
                .ok();
            ok = true;
            continue;
        }
        
        tx.send(format!("..  {}  patching DLL...", inst.version)).ok();
        
        // 根据 is_tuanjie 标志选择补丁方法
        let patch_result = if inst.is_tuanjie {
            core::patch_tuanjie_dll(&inst.dll_path)
        } else {
            core::patch_dll(&inst.dll_path)
        };
        
        match patch_result {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL patched", inst.version)).ok();
                // ... exe patching ...
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", inst.version, e)).ok();
            }
        }
    }
    
    // ... rest of function ...
}
```

## Shared Licensing Client 迁移

### 现状

| 位置 | 路径 | 状态 |
|------|------|------|
| Unity Hub | `C:\Program Files\Unity Hub\UnityLicensingClient_V1\` | ✅ 支持 |
| Tuanjie Hub | `C:\Program Files\Tuanjie Hub\TuanjieLicensingClient_V1\` | ❌ 未支持 |

### DLL 对比

| 对比项 | Unity Shared Client | Tuanjie Shared Client |
|--------|---------------------|----------------------|
| 文件名 | `Unity.Licensing.EntitlementResolver.dll` | `Tuanjie.Licensing.EntitlementResolver.dll` |
| 体积 | ~211 KB | ~211 KB |
| 与 Editor DLL 对比 | 字节一致 | 字节一致 |

### 迁移策略

Tuanjie Hub 的 Shared Licensing Client 与 Tuanjie Editor DLL **字节完全一致**，采用相同的 .NET 嵌入资源机制。迁移方案：

1. **复用 Editor 补丁逻辑**: `patch_shared_licensing_client()` 扩展为同时检测 Unity 和 Tuanjie 路径
2. **统一 .NET 资源替换**: 使用相同的 Mono.Cecil 工具处理两个 DLL

```rust
// src/core/binary_patch.rs
pub fn patch_shared_licensing_client() -> anyhow::Result<()> {
    let unity_dll = Path::new(r"C:\Program Files\Unity Hub\UnityLicensingClient_V1\Unity.Licensing.EntitlementResolver.dll");
    let tuanjie_dll = Path::new(r"C:\Program Files\Tuanjie Hub\TuanjieLicensingClient_V1\Tuanjie.Licensing.EntitlementResolver.dll");
    
    if unity_dll.exists() {
        patch_dll(&unity_dll)?; // 字节替换
    }
    
    if tuanjie_dll.exists() {
        patch_tuanjie_dotnet_resource(&tuanjie_dll)?; // .NET 资源替换
    }
    
    Ok(())
}
```

## 扫描逻辑扩展

### Editor 扫描

```rust
// src/core/install.rs

#[derive(Clone)]
pub struct UnityInstall {
    pub version: String,
    pub dll_path: PathBuf,
    pub exe_path: PathBuf,
    pub is_tuanjie: bool,  // 新增字段
}

pub fn scan_installations() -> Vec<UnityInstall> {
    let mut installs = Vec::new();

    // 扫描 C:\Program Files\Unity * 和 Tuanjie *
    if let Ok(dir) = std::fs::read_dir(r"C:\Program Files") {
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            
            if path.is_dir() {
                // Unity Editor
                if name.starts_with("Unity ") {
                    let version = name.strip_prefix("Unity ").unwrap_or(&name);
                    if let Some(install) = try_find_install(&path, version) {
                        installs.push(install);
                    }
                }
                // Tuanjie Editor
                else if name.starts_with("Tuanjie ") {
                    let version = name.strip_prefix("Tuanjie ").unwrap_or(&name);
                    if let Some(install) = try_find_tuanjie_install(&path, version) {
                        installs.push(install);
                    }
                }
            }
        }
    }

    // 扫描 Hub 安装的 Editor
    for hub_editor_dir in &[
        r"C:\Program Files\Unity Hub\Editor",
        r"C:\Program Files\Tuanjie Hub\Editor",  // 新增
    ] {
        if let Ok(dir) = std::fs::read_dir(hub_editor_dir) {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let version = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "UNKNOWN".to_string());
                    
                    // 检测是 Unity 还是 Tuanjie
                    let is_tuanjie = hub_editor_dir.contains("Tuanjie");
                    let install = if is_tuanjie {
                        try_find_tuanjie_install(&path, &version)
                    } else {
                        try_find_install(&path, &version)
                    };
                    
                    if let Some(install) = install {
                        installs.push(install);
                    }
                }
            }
        }
    }

    installs.sort_by(|a, b| b.version.cmp(&a.version));
    installs
}

fn try_find_tuanjie_install(root: &Path, version: &str) -> Option<UnityInstall> {
    let dll_path = root.join(r"Editor\Data\Resources\Licensing\Client\Tuanjie.Licensing.EntitlementResolver.dll");
    if dll_path.exists() {
        Some(UnityInstall {
            version: version.to_string(),
            dll_path,
            exe_path: root.join(r"Editor\Tuanjie.exe"),
            is_tuanjie: true,
        })
    } else {
        None
    }
}
```

## 实施计划

### 阶段 1: Tuanjie Hub 适配 (低风险, 高收益)

**目标**: 支持 Tuanjie Hub 1.4.2 ASAR 补丁

**任务**:
1. 新增 `src/hub/versions/tuanjie.rs` 实现 `TuanjieStrategy`
2. 扩展 `find_hub()` 支持 Tuanjie Hub 路径
3. 扩展注册表查找支持 `Tuanjie Hub`
4. 调整 `getValidEntitlementGroups` 补丁目标文件
5. 重新实现 AuthService 登录状态注入逻辑
6. 测试验证补丁效果

**预计工作量**: 2-3 天

**验收标准**:
- Tuanjie Hub 启动后显示已登录状态
- 可以正常下载和安装 Tuanjie Editor
- 许可证验证通过

### 阶段 2: Tuanjie Editor DLL 补丁工具 (中风险, 中收益)

**目标**: 实现 .NET 资源替换工具，支持 Tuanjie Editor DLL 补丁

**任务**:
1. 提取 Tuanjie DLL 内嵌证书 (DER 格式)
2. 生成修改版证书 + 配套私钥
3. 评估 Mono.Cecil vs dnlib vs ilspycmd
4. 实现 .NET 资源替换工具 (C# 或 Rust FFI)
5. 集成到 `patch_dll()` 流程
6. 测试验证补丁效果

**预计工作量**: 5-7 天

**验收标准**:
- Tuanjie Editor 启动后许可证验证通过
- 不报 "digital signature is invalid" 错误
- 可以正常激活和使用

### 阶段 3: Shared Licensing Client 统一支持 (低风险)

**目标**: 扩展 `patch_shared_licensing_client()` 同时支持 Unity 和 Tuanjie

**任务**:
1. 扩展路径检测逻辑
2. 根据 DLL 类型选择补丁方法 (字节替换 vs .NET 资源替换)
3. 测试验证

**预计工作量**: 1 天

**验收标准**:
- Tuanjie Hub 安装的 Shared Licensing Client 正常补丁
- Unity 2021 Editor IPC 验证通过

### 阶段 4: UI 和日志适配 (低风险)

**目标**: UI 正确显示 Tuanjie 相关组件，日志清晰区分 Unity/Tuanjie

**任务**:
1. UI 列表显示 Tuanjie Hub / Tuanjie Editor 标识
2. 补丁日志输出区分 Unity/Tuanjie
3. 错误提示信息适配

**预计工作量**: 0.5 天

## 技术风险

### 风险 1: .NET 资源替换失败

**原因**: Mono.Cecil 可能无法正确处理某些 .NET 程序集特性

**缓解措施**:
- 提前测试 Mono.Cecil 对 Tuanjie DLL 的兼容性
- 准备 dnlib 作为备选方案
- 保留 dnSpy / ILSpy 手动修改路径

### 风险 2: 证书签名验证失败

**原因**: Tuanjie 可能使用更严格的证书链验证

**缓解措施**:
- 测试自签名证书是否被接受
- 如需 DigiCert 签发，提前申请证书
- 准备 Hook 方案作为后备

### 风险 3: ASAR 结构变化

**原因**: Tuanjie Hub 版本更新可能导致 JS 结构变化

**缓解措施**:
- 实现版本检测逻辑，根据版本号选择补丁策略
- 添加详细的函数签名匹配日志
- 保留手动修改 ASAR 的能力

### 风险 4: 反作弊检测

**原因**: Tuanjie 可能集成反作弊系统检测 DLL 修改

**缓解措施**:
- 优先使用 .NET 资源替换而非 IL 修改
- 避免修改关键校验函数
- 准备运行时 Hook 方案

## 测试策略

### Hub 测试

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_tuanjie_hub_detection() {
        let hub = find_hub();
        assert!(hub.is_some());
        let info = hub.unwrap();
        assert!(info.version.contains("1.4.2"));
        assert!(info.asar_path.to_string_lossy().contains("Tuanjie"));
    }
    
    #[test]
    fn test_tuanjie_strategy_applies() {
        let asar = read_asar(Path::new(r"C:\Program Files\Tuanjie Hub\resources\app.asar")).unwrap();
        let strategy = detect_strategy(&asar).unwrap();
        assert_eq!(strategy.version_name(), "Tuanjie Hub (CommonJS)");
    }
    
    #[test]
    fn test_tuanjie_hub_patch_roundtrip() {
        let hub = find_hub().unwrap();
        let bak = hub.asar_path.with_extension("asar.bak");
        
        // 备份
        std::fs::copy(&hub.asar_path, &bak).unwrap();
        
        // 补丁
        patch_hub(&hub).unwrap();
        assert!(is_hub_patched(&hub.asar_path));
        
        // 还原
        unpatch_hub(&hub).unwrap();
        assert!(!is_hub_patched(&hub.asar_path));
    }
}
```

### Editor 测试

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_tuanjie_editor_detection() {
        let installs = scan_installations();
        let tuanjie = installs.iter().find(|i| i.version.contains("2022.3.62t8"));
        assert!(tuanjie.is_some());
        assert!(tuanjie.unwrap().is_tuanjie);
    }
    
    #[test]
    fn test_tuanjie_dll_patch() {
        let dll = Path::new(r"C:\Program Files\Tuanjie 2022.3.62t8\Editor\Data\Resources\Licensing\Client\Tuanjie.Licensing.EntitlementResolver.dll");
        if !dll.exists() {
            eprintln!("SKIP: Tuanjie Editor not installed");
            return;
        }
        
        let bak = dll.with_extension("dll.bak");
        std::fs::copy(dll, &bak).unwrap();
        
        patch_tuanjie_dll(dll).unwrap();
        assert!(is_tuanjie_patched(dll));
        
        unpatch_tuanjie_dll(dll).unwrap();
        assert!(!is_tuanjie_patched(dll));
    }
}
```

### 集成测试

```powershell
# 完整流程测试
cargo build --release

# 1. 补丁 Tuanjie Hub
.\target\release\unifuck.exe --patch-hub

# 2. 启动 Tuanjie Hub，验证登录状态
Start-Process "C:\Program Files\Tuanjie Hub\Tuanjie Hub.exe"
Start-Sleep -Seconds 10
# 手动检查 UI 显示已登录

# 3. 安装 Tuanjie Editor (通过 Hub)
# 手动操作...

# 4. 补丁 Tuanjie Editor
.\target\release\unifuck.exe --patch-editor "2022.3.62t8"

# 5. 启动 Tuanjie Editor，验证许可证
Start-Process "C:\Program Files\Tuanjie 2022.3.62t8\Editor\Tuanjie.exe"
# 手动检查许可证状态

# 6. 还原所有补丁
.\target\release\unifuck.exe --unpatch-all
```

## 依赖评估

### 新增 Rust 依赖

```toml
[dependencies]
# 无需新增，复用现有依赖
```

### 新增 .NET 依赖 (如采用方案 A)

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  
  <ItemGroup>
    <PackageReference Include="Mono.Cecil" Version="0.11.5" />
  </ItemGroup>
</Project>
```

### 工具链依赖

| 工具 | 用途 | 必要性 |
|------|------|--------|
| Mono.Cecil | .NET 程序集操作 | 高 (方案 A) |
| dnlib | .NET 程序集操作 (备选) | 中 |
| ilspycmd | .NET 反编译 | 低 (仅调试) |
| dnSpy | 手动修改 .NET 程序集 | 低 (仅调试) |

## 总结

Tuanjie Hub 补丁迁移**可行性高**，仅需调整 JS 补丁目标文件和登录状态注入逻辑，预计 2-3 天完成。

Tuanjie Editor DLL 补丁迁移**需要切换技术路线**，从字节替换改为 .NET 资源替换，预计 5-7 天完成。推荐采用 Mono.Cecil 方案，保持 PE 结构完整性。

建议按阶段实施，优先完成 Hub 适配 (低风险高收益)，再推进 Editor DLL 补丁工具 (中风险中收益)。
