# Tuanjie License System 安全研究报告

## 摘要

研究对象: Tuanjie Hub 1.4.2 (团结引擎 Hub) 与 Tuanjie Editor 2022.3.62t8 (团结引擎编辑器) 的 license 验证系统在离线部署场景下的实施强度。Tuanjie 是 Unity 中国分支,独立公司,其 license 系统继承 Unity 体系 (Electron Hub + .NET EntitlementResolver + ULF XML DSig),额外引入基于 Sentinel HASP (Thales 商用 DRM) 的 native IPC AuthorizationCheck 协议层。报告识别了从 Hub 登录态、Editor .NET 公钥嵌入、ULF 签名、IPC 对称密钥协议、Sentinel HASP envelope 配置到生产 PDB 发布等多个层面的实施弱点。研究以内部资产风险评估为目的展开,对每个弱点给出技术细节、利用方法与缓解建议。整体评估: Tuanjie 比 Unity 多了一层 native IPC 防护,但 Sentinel HASP envelope 的配置偏弱、对称式 IPC 协议使 server/verifier 密钥共享成为单点失效、生产 PDB 的公开分发进一步降低了运行时逆向与 hook 的成本。

| 维度 | 评级 | 说明 |
|------|------|------|
| 信息泄漏 | 低 | PDB 公开分发暴露完整 native 符号表,属于工程实践缺陷 |
| 完整性 | 高 | 多层信任锚点可被替换: Hub JS → .NET cert → ULF sig → IPC token → native verifier code patch |
| 所需权限 | 管理员 | 写 Program Files / ProgramData / hosts |
| PoC 成熟度 | 已实现 | 全链代码已完成 (Hub + .NET + ULF + loader cdylib),Sentinel 防御响应需实测确认 |

## 影响范围

| 组件 | 版本 | 路径 | 格式 |
|------|------|------|------|
| Tuanjie Hub | 1.4.2 | `C:\Program Files\Tuanjie Hub` | Electron ASAR (CommonJS) |
| EntitlementResolver | - | `Editor\Data\Resources\Licensing\Client\Tuanjie.Licensing.EntitlementResolver.dll` | .NET DLL (PEM 公钥在 #US 堆,UTF-16LE CRLF) |
| Tuanjie Editor | 2022.3.62t8 | `C:\Program Files\Tuanjie 2022.3.62t8\Editor\Tuanjie.exe` | Native x64, Sentinel HASP enveloped |
| bindings.dll | - | `Tuanjie Hub\TuanjieLicensingClient_V1\bindings.dll` | Native x64, Sentinel HASP enveloped |
| ProgramData ULF | - | `C:\ProgramData\Tuanjie\Tuanjie_lic.ulf` | XML DSig RSA-SHA1 |
| 云端 | `license.tuanjie.cn` / `api.tuanjie.cn` | HTTPS | 在线验证 |

## 系统架构

Tuanjie license 系统在 Unity 体系之上增加了一道 native IPC AuthorizationCheck 协议。全链路共五层:

```
Tuanjie Hub (Electron ASAR)
    ├─ AuthService.js         isUserSignedIn / init / setLoggedInFlags
    ├─ licenseService.js      isLicenseValid / getValidEntitlementGroups
    ├─ licensingSdk.js         isUseService / init / getInstance
    │                          (isUseService 为 Tuanjie 专有)
    ▼ [Named Pipe IPC]
Tuanjie.Licensing.Client.exe (.NET)
    ├─ Controller<T>.MessageAuthorizationCheck
    │   → [DllImport("bindings")] VerifyLicenseFiles(nonce, ulfPaths[], n, messageType)
    │   bindings.dll (Sentinel HASP enveloped)
    │     ├─ 读 ULF 文件 + 内置对称密钥 → 计算 HMAC/MAC token
    │     └─ 返回 ANSI char* (= authorizationCheck token)
    ▼ [IPC Response JSON 含 authorizationCheck 字段]
Tuanjie.exe (Editor, Sentinel HASP enveloped)
    ├─ 生成 nonce (BCryptGenRandom, ~11 bytes, base64 编码)
    ├─ 收到 response 后调 IsValidIPCResponse(nonce, token) → bool
    │    本地用同一对称密钥重算 token 并比对
    │    false → "[Licensing::Client] AuthorizationCheck is invalid" → 拒绝
    ├─ ValidateServerProcess() → bool (并行 guard,校验 .NET Client 进程身份)
    └─ Licensing::Module 分发 entitlement granted/denied
```

与 Unity 的关键差异:

- Tuanjie Editor 主程序 `Tuanjie.exe` 被 Sentinel HASP envelope (Thales 商用 DRM) 包裹,`.text`/`.rdata`/`.data` 三个段在磁盘上 AES 加密 (entropy 8.000),运行时由 `.merged` 段的 Wrapper stub 解密
- .NET Client 使用的 native 模块 `bindings.dll` 同样被 Sentinel HASP 包裹,其导出函数 `VerifyLicenseFiles` 承担 IPC token 计算职责
- Editor 与 .NET Client 之间通过 IPC AuthorizationCheck 进行对称握手,Editor 本地 `IsValidIPCResponse` 担当最终验证
- Hub 中新增 `isUseService()` 函数 — Tuanjie 自有的云端服务开关
- ULF 公钥在 EntitlementResolver.dll 中以 PEM 文本存储,编码 UTF-16LE CRLF (不同于 Unity 的 base64 DER)
- `Tuanjie.exe` 同目录公开分发完整未 strip 的 PDB `Tuanjie_x64.pdb` (104 MB)

---

## 弱点一: Hub ASAR 无强制完整性 + JS 全本地判定 (同 Unity 模式)

### 机制

Tuanjie Hub 使用与 Unity Hub 相同的 Electron + ASAR 架构。登录态 (`isUserSignedIn`)、license 校验 (`isLicenseValid`)、SDK 初始化均在本地 JS 中执行。Tuanjie 新增的 `isUseService()` 控制云端服务开关,同样可被静态覆写。ASAR 未通过 Electron Fuses 强制校验,解包 + 改 JS + 重打包即可生效。

### 利用

对 `AuthService.js` 的 3 个函数与 `licensingSdk.js` 的 2 个函数及 `licenseService.js` 逐一替换:

**`AuthService.js`**: 覆盖 `init()` 使其跳过 `onNetworkUp()` 中的 logout 路径,直接调用 `setLoggedInFlags()` 设置已登录标记。`isUserSignedIn()` 返回 `true`。`constructor()` 注入伪造 `userInfo` 和 `connectInfo`。

**`licensingSdk.js`**: 覆盖 `init()` 跳过网络校验返回 `true`。覆盖 `isUseService()` 返回 `false` 使 Hub 不依赖云端 API。覆盖 `getInstance()` 返回 fat stub,包含所有方法的零风险默认值 (空字符串/Promise.resolve(false)/空数组),避免未初始化的 SDK 在调用链上抛出 NullReference 导致 Electron 进程闪退。

**`licenseService.js`**: 覆盖 `isLicenseValid()` 返回 `true`。覆盖 `getValidEntitlementGroups()` 注入伪造的 Pro 授权对象,使 Hub UI 显示有效 license。

### 影响

Hub 的登录/授权状态完全由本地 JS 决定,离线场景下无任何服务端回溯。

**严重度: 高。**

---

## 弱点二: 域名屏蔽阻断云端 (同 Unity 模式)

### 机制

Tuanjie Hub / Editor 启动时向 `license.tuanjie.cn`、`core.cloud.tuanjie.cn`、`api.tuanjie.cn` 发起 HTTPS 请求,用于在线 license 签发与心跳。这些域名通过标准 DNS 解析,解析结果受 Windows hosts 文件控制。未发现 Hub 实施离线超时宽限期逻辑。

### 利用

向 `C:\Windows\System32\drivers\etc\hosts` 追加 127.0.0.1 映射,阻断所有在线路径,Hub / Editor 降级为离线模式,信任本地 ULF 文件。

### 影响

阻断域名使云端无法线上拉黑或撤销 license,且无心跳超时自动失效机制。

**严重度: 中。**

---

## 弱点三: EntitlementResolver 公钥以 PEM 文本嵌入 (UTF-16LE CRLF)

### 机制

`Tuanjie.Licensing.EntitlementResolver.dll` 中的 ULF 签名公钥以完整的 PEM 字符串字面量形式存储在 .NET #US 堆。PEM 文本含 `-----BEGIN CERTIFICATE-----` header、64 字符一行的 base64 体、`-----END CERTIFICATE-----` footer。编码为 UTF-16LE CRLF — Windows 上 C# 编译器将源文件中的 CRLF 行尾原样保留在 #US 堆字符串常量中。

### 利用

1. 提取原始 PEM 文本 → 将行尾标准化为 CRLF → 编码为 UTF-16LE 字节序列
2. 自签一个新的 RSA 4096 公钥,PEM 编码,同样转 UTF-16LE CRLF
3. 确保原始与自签的 UTF-16LE 字节序列长度严格相等。PEM 的每行长度可通过调整证书 Subject 字段 padding 来对齐
4. 在 DLL 二进制字节中搜索原始 UTF-16LE 序列,找到后原地 `memcpy` 替换

与 Unity cert 替换的区别: Unity 用的 base64 DER (不含 PEM header/footer),Tuanjie 用完整 PEM,且编码是 UTF-16LE 而非 ASCII,搜索空间更大 (每字符 2 字节)。但由于 DLL 体积小 (~2 MB),全扫描时间可忽略。替换不触及 PE header 或 .NET 元数据,不产生偏移量变化。

### 影响

公钥是 ULF 签名信任链唯一锚点,替换公钥使自签 ULF 可被 Editor 接受。

**严重度: 高。**

---

## 弱点四: ULF 签名 SHA-1 + 仅覆盖 Terms 子树

### 机制

与 Unity 相同: W3C XML DSig RSA-SHA1, Inclusive C14N WithComments,仅覆盖 `<License id="Terms">` 子树而非整个 ULF 文档。SHA-1 已被 NIST 淘汰。

### 利用

自签 ULF 流程: 准备模板 → C14N Terms 子树 → SHA-1 hash → base64 填 `<DigestValue>` → C14N SignedInfo → RSA-SHA1 签名 → base64 填 `<SignatureValue>` → 部署到 ProgramData。

验证: 自签 ULF 的 `<DigestValue>` 与 Tuanjie 部署版 bit-identical (独立 lxml C14N 确认);签名字段经 PKCS#1 v1.5 RSA-SHA1 数学验签通过;audit log 中原样输出 `granted: True` for `legacy.pro / ui / headless`。

### 影响

.NET EntitlementResolver 完全接受 patched cert + 自签 ULF。SHA-1 的碰撞风险和子树签名的覆盖不足降低了 ULF 签名的有效性,但需先突破公钥存储 (弱点三) 才能利用。ULF 不是阻塞点 — 真正的阻塞在下层 native IPC。

**严重度: 中。**

---

## 弱点五: IPC AuthorizationCheck 采用对称密钥协议

### 机制

这是 Tuanjie 相比 Unity 新增的防护层,架构存在结构性设计缺陷。

**协议流程**:

1. Editor (Tuanjie.exe) 调用 `BCryptGenRandom` 生成 11 字节 nonce,base64 编码后填入 IPC 请求 JSON 的 `AuthorizationCheck` 字段
2. .NET LicensingClient (`Tuanjie.Licensing.Client.exe`) 的 `Controller<T>.MessageAuthorizationCheck` 将 nonce、ULF 文件路径数组、消息类型名传入 `[DllImport("bindings")] VerifyLicenseFiles(nonce, ulfPaths[], n, messageType)`
3. `bindings.dll` (Sentinel HASP enveloped, 15.5 MB) 内部使用共享对称密钥 + ULF 文件内容指纹计算 HMAC/AEAD tag (~8-11 字节),作为 ANSI 字符串返回
4. .NET 将该 token 不经修改填入 Response JSON 的 `authorizationCheck` 字段,发回 Editor
5. Editor 的 `Licensing::LicensingClient::IsValidIPCResponse(nonce, token)` 本地使用同一对称密钥重算 token 并比对
6. 若比对失败: `"[Licensing::Client] AuthorizationCheck is invalid"` → `"LicensingClient has failed validation; ignoring"` → 拒绝发放 license

**为什么这是设计缺陷**: 对称协议要求 server (.NET Client 通过 bindings.dll) 与 verifier (Editor.exe) 必须共享相同密钥。一旦 Editor 端的 verifier 被内存 patch 使其无条件返回 `true`,整个 token 校验层被绕过。非对称设计 (Client 用私钥签 nonce,Editor 用公钥验) 不存在此问题 — 即使 Editor 端被 compromise,攻击者也无法伪造 Client 的签名。

### 利用

通过内存 patch Editor 端的 native verifier `IsValidIPCResponse`,使其无条件返回 `true`,绕过整个 token 校验层。利用方式详见弱点六、八。

在一次完整离线启动中捕获的日志时间线确认:

```
15:25:43.666  .NET EntitlementResolver successfully parsed ULF       ← ULF 层 OK
15:25:43.886  Editor IPC handshake OK                                 ← 连接 OK
15:25:45.573  audit: legacy.pro granted True                          ← .NET 端判定通过
15:25:45.574  audit: ui granted True
15:25:45.595  .NET sends EntitlementDetailsResponse [200]             ← response 发送
15:25:45.613  Editor.log: [Licensing::Client] AuthorizationCheck is invalid  ← native 拒绝
15:25:45.620  Editor.log: No valid Unity Editor license found
```

结论: .NET 端 (EntitlementResolver + audit + controller) 全部成功;失败点在 Editor 端的 native IPC verifier `IsValidIPCResponse`。

### 影响

对称协议要求 server 与 verifier 共享同一密钥。Editor 端的 verifier 被内存 patch 使无条件返回 `true` 后,整个 token 校验层被绕过。非对称设计 (Client 私钥签 nonce,Editor 公钥验) 不存在此问题 — 即使 Editor 端被 compromise,攻击者也无法伪造 Client 的签名。

**严重度: 高。**

---

## 弱点六: Tuanjie 公开分发完整未 strip 的生产 PDB

### 机制

`C:\Program Files\Tuanjie 2022.3.62t8\Editor\Tuanjie_x64.pdb` (104 MB) 是完整的 public symbol PDB,包含 `Licensing::*` namespace 下 1642 个 native 符号的 mangled name、section、offset、长度信息。关键暴露:

- `?IsValidIPCResponse@LicensingClient@Licensing@@QEAA_NAEBV?$basic_string@DV?$StringStorageDefault@D@core@@@core@@0@Z` — verifier 入口,含 RVA
- `?ValidateServerProcess@LicensingClient@Licensing@@...` — 并行 guard,含 RVA
- 全部 `Licensing::Module::*` / `Licensing::IPC::*` 类成员
- 内部错误字符串常量的 symbol table 条目 (如 `"AuthorizationCheck is invalid"`),可实现字符串交叉引用定位回调点

通过 PDB 的 `address_map()` 可将每个符号的 `PdbInternalSectionOffset` 转换为标准 RVA;加 ImageBase 即得运行时绝对地址。这使得定位到 IPC verifier 函数入口的逆向成本从"需要动态分析 / 模式扫描"降到"解析 PDB + 一次 VirtualProtect 写入"。

生产环境分发未 strip 的完整 PDB 违反了"仅交付 stripped 符号"的工程惯例。

### 利用

伪代码:

```
pdb = PDB::open("Tuanjie_x64.pdb")
symbols = pdb.global_symbols()
addr_map = pdb.address_map()

for symbol in symbols:
    name = symbol.parse()
    if name == "?IsValidIPCResponse@LicensingClient@Licensing@@QEAA_N...":
        rva = addr_map.rva(symbol.offset)
        target_va = GetModuleHandle(NULL) + rva
        // 等 Sentinel 解密 .text 后,写 4 字节 stub
        VirtualProtect(target_va, 4, PAGE_EXECUTE_READWRITE, &old)
        memcpy(target_va, [0xB0, 0x01, 0xC3, 0x90])  // mov al,1; ret; nop
        VirtualProtect(target_va, 4, old, &tmp)
        FlushInstructionCache(GetCurrentProcess(), target_va, 4)
```

同时比对 PDB header 中的 GUID/age 与 PE DEBUG_DIRECTORY 中的 RSDS 记录,确保 PDB 与运行的 exe 版本一致,防止 RVA 错位。

### 影响

不直接暴露密钥材料,但将定位到 IPC verifier 函数入口的逆向成本从"需要 attach 调试器 + 分析加密 .text"降到"解析 PDB + 一次 VirtualProtect 写入",降低至少一个量级。属于加固缺失 (hardening omission) 而非设计缺陷。

**严重度: 中。**

---

## 弱点七: DLL Forwarder Hijack 入口存在

### 机制

`Tuanjie.exe` 的静态 IAT 中列有 `VERSION.dll!VerQueryValueW`。`version.dll` 不在 Windows KnownDLLs 注册表键中 (`HKLM\...\KnownDLLs` 实测不含 `version`),且 `C:\Program Files\Tuanjie 2022.3.62t8\Editor\` 目录下无同名 DLL。当我们在 Editor 目录放置名为 `version.dll` 的模块时,Windows PE Loader 按 SafeDllSearchMode 优先从当前目录加载,再回退到 System32。

`version.dll` 仅有 17 个命名导出 (全部 `GetFileVersionInfo*` / `Ver*` / `VerQueryValue*` 系列),Tuanjie.exe 仅 import 其中一个 (`VerQueryValueW`)。作为 forwarder 的实现成本极低。

### 利用

编写一个 cdylib,在运行时加载真正的 `C:\Windows\System32\version.dll`,将所有 17 个导出转发到真实函数。DllMain 的 `DLL_PROCESS_ATTACH` 中起一条独立 worker 线程,worker 负责等 Sentinel 解密完成后对 native verifier 做内存 patch。主线程立即返回 TRUE,不阻塞进程加载。

**为何不写 PE 级 forward string (`/EXPORT:Foo=version_orig.Foo`)**: 因为我们的 DLL 名为 `version.dll`,forward 字符串写 `version_orig.VerQueryValueW` 要求旁置一份改名的真 DLL。runtime trampoline (`LoadLibrary` + `GetProcAddress` 缓存 + tail-call) 方案不需要拷贝 system32 文件,部署更干净。

伪代码 (forwarder 骨架):

```c
static HMODULE real_dll;

BOOL WINAPI DllMain(HINSTANCE hinst, DWORD reason, LPVOID reserved) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(hinst);
        CreateThread(NULL, 0, worker_thread, NULL, 0, NULL);
    }
    return TRUE;
}

// 每个 forwarder 函数:
static VerQueryValueW_t real_VerQueryValueW;
BOOL WINAPI VerQueryValueW(const void* block, const WCHAR* sub,
                           void** buf, UINT* len) {
    if (!real_VerQueryValueW) {
        if (!real_dll) real_dll = LoadLibraryW(L"C:\\Windows\\System32\\version.dll");
        real_VerQueryValueW = (VerQueryValueW_t)GetProcAddress(real_dll, "VerQueryValueW");
    }
    return real_VerQueryValueW(block, sub, buf, len);
}
// ...16 个其他函数同样模式
```

### 影响

DLL hijack 是已知通用攻击面,但 Tuanjie 未采取防御措施: 同目录 DLL 无签名前置检查、无 IFEO (Image File Execution Options) 自检、version.dll 不在 KnownDLLs 注册。Windows Loader 默认的 SafeDllSearchMode 优先从当前目录加载,使攻击者可将任意代码注入 Editor 进程。

**严重度: 中。**

---

## 弱点八: Sentinel HASP Envelope 对 ISV .text 周期自检缺失

### 机制

`Tuanjie.exe` 被 Sentinel HASP envelope 包裹:

- `.text` (78 MB) entropy 8.000 — 磁盘全加密
- `.rdata` (15 MB) entropy 8.000
- `.data` (7 MB) entropy 8.000
- `.merged` (13 MB, RWX) — Sentinel wrapper stub,解密 + IAT 重建 + anti-debug 逻辑

启动时 wrapper stub 执行解密 → IAT 重建 → integrity self-check → 跳转到原始 OEP。此后 `.text` 在内存中为 plaintext RX 页。

Sentinel HASP envelope 的公开 anti-tamper 防护主要覆盖:

1. Disk binary modification — Authenticode 签名校验 (触发 `Disable key for attempted tampering`)
2. Encryption engine tampering — wrapper 自身代码 (`.merged`) 被改动
3. Active debugger detection — `IsDebuggerPresent` / `CheckRemoteDebuggerPresent` / hardware breakpoint 检查
4. License container enforcement — v2c/alf/h2r 格式 + RSA pubkey 验签

以上均可通过 in-memory patch 规避: 不动磁盘 → 签名完整; 不碰 `.merged`; 不挂调试器; 不调 hasp_*/sntl_* API。

**对 ISV 自有 `.text` 段是否做周期 CRC/HMAC 复检**: Thales Sentinel LDK 公开文档未确认此功能;社区逆向实践 (Tuts4You, exetools, reteam) 多年无公开实证。对比同类壳: VMProtect 2 有 `CHECK_CODE_INTEGRITY` VM handler 做全 `.text` 散列,Themida 3.x 有周期 CRC 线程,但 Sentinel HASP 的定位是 license 保护 (非 anti-cheat),没有类似公开机制。

**`Disable key for attempted tampering` 风险**: 如果 Tuanjie 在打包 envelope 时启用了此选项且使用的是 Sentinel SL 8.51+ 或 HL 4.54+ 密钥,任一 trigger 命中后 Sentinel 会将 license key 在本地 store (`%ProgramData%\SafeNet Sentinel\Sentinel LDK\`) 中写入 disabled 状态。当前 in-memory patch 方案理论上不命中任一公开 trigger,但若 Sentinel 内部有未公开的隐藏自检路径,首次部署有极小概率触发。此时可删除 Sentinel store + 重新部署 ULF 恢复激活,不可逆风险可控。

### 利用

Worker 线程在 DllMain 之后,等 wrapper 完成解密的策略: 从 PDB 解出 `IsValidIPCResponse` 的 RVA → 周期读取该地址 16 字节 → 用 prologue 字节模式判定是否已解密:

判定规则:

```
function looks_like_x64_prologue(bytes[16]) -> bool:
    // 1. 排除密文: 不是全 0
    if all bytes == 0: return false
    // 2. 排除填充: 不是全 0xCC
    if all bytes == 0xCC: return false
    // 3. 排除高熵随机 = 仍加密: entropy(16 bytes) < 7.5
    // 4. 首字节属于 x64 常见 prologue 前缀族:
    //    0x40-0x4f (REX), 0x48-0x4f (REX.W), 0x55 (push rbp),
    //    0x53 (push rbx), 0x57 (push rdi)
    if first_byte not in {0x40..0x4f, 0x48..0x4f, 0x55, 0x53, 0x57}: return false
    // 5. 前 8 字节内出现 MSVC 标准 prologue 模式之一:
    //    sub rsp, imm: 0x48 0x83 0xEC ?? 或 0x48 0x81 0xEC ?? ?? ?? ??
    //    home spill:   0x48 0x89 [0x5c|0x74|0x7c|0x4c] 0x24 ??
    //                   即 mov [rsp+off], reg64
```

在 worker 中 50ms 周期轮询,设 30s 超时。命中後:

```
VirtualProtect(target_va, 4, PAGE_EXECUTE_READWRITE, &old_protect);
copy 4 bytes [0xB0, 0x01, 0xC3, 0x90] to target_va;
VirtualProtect(target_va, 4, old_protect, &ignored);
FlushInstructionCache(GetCurrentProcess(), target_va, 4);
```

对 `ValidateServerProcess` 同理操作。两个 4 字节 stub 写完后 worker 退出,不设全局 hook、不留长期循环。Sentinel 在解密完成后不监视 .text 页属性变更,也不对 ISV 代码区做周期散列,因此 patch 可稳定存活到进程退出。

### 影响

Sentinel HASP envelope 提供了可观的静态逆向壁垒 (`.text` 加密 + IAT 分解 + anti-debug),但由于未启用周期自检,该壁垒在运行时被限制为一层"一次性解密"保护。一旦加载到解密后的内存,攻击者可在用户态自由修改 ISV 代码页而不被 envelope 检测。整个 IPC AuthorizationCheck 的防御力依赖于 Sentinel 自身不被绕过,而 Sentinel 仅保护了自己,未保护 Tuanjie 的代码。

**严重度: 中。** 不是 Sentinel 产品的安全漏洞,而是 Tuanjie 在配置 Sentinel 时未启用所有可用的代码完整性保护,且未补充应用层自检。

---

## 利用链路

| 步骤 | 动作 | 依赖弱点 |
|------|------|---------|
| 1 | 自签 RSA 4096 密钥对 | — |
| 2 | 替换 EntitlementResolver.dll PEM 公钥 (UTF-16LE CRLF 搜索+替换) | 弱点三 |
| 3 | 自签 ULF → XML DSig RSA-SHA1 → 部署到 ProgramData | 弱点四 |
| 4 | Hub ASAR patch (AuthService / licensingSdk / licenseService) | 弱点一 |
| 5 | hosts 屏蔽 tuanjie.cn 域名 | 弱点二 |
| 6 | 编译 cdylib `version.dll` (17 个 forwarder + DllMain + worker) | 弱点七 |
| 7 | 部署 forwarder 到 `Editor\version.dll` | 弱点七 |
| 8 | 启动 Tuanjie → worker 等 Sentinel 解密 → PDB 解 RVA → 内存 patch `IsValidIPCResponse` + `ValidateServerProcess` → return true → IPC 通过 | 弱点五、六、八 |
| 9 | license granted, Editor 正常启动 | — |

回滚: 步骤 2/4/6 均在对目标文件修改前创建 `.bak`;步骤 3 删除 ULF;步骤 5 清除 hosts 块;步骤 7 删除 forwarder DLL 并还原 `.bak`。所有操作可逆。

**关键安全属性: 所有写操作先做 `.bak` 备份,支持一键恢复。**

---

## 缓解建议

按加固成本从低到高排列。

### 短期

**不公开发布未 strip 的生产 PDB**: 使用 stripped PDB (仅保留公共 crash 符号子集),或在 release build 中完全移除 PDB。当前发布策略将 native 逆向成本从"需要 attach 调试器 + 分析 .text"直接降到"查 PDB + 写 4 字节"。

**IPC 协议从对称密钥改为非对称签名**: 将 `VerifyLicenseFiles` + `IsValidIPCResponse` 的对称 HMAC 方案替换为:

1. Editor 生成 nonce + 临时密钥对,EphPubKey 与 nonce 一同发给 Client
2. Client 调用 `VerifyLicenseFiles` 用内置私钥对 `(nonce, ULF_file_hash, messageType)` 做 ECDSA/RSA 签名
3. Editor 用内嵌公钥验证签名 (公钥不需要对称共享,只需嵌入在 Editor 中由 Sentinel 保护)

对称到非对称的切换,从根上消除"server/verifier 共享密钥单点失效"问题。

### 中期

- **EntitlementResolver 公钥存储加固**: PEM 文本 → 加密资源 (key 绑定 machine SID + DPAPI),运行时解密。同时对 DLL 增加 Authenticode 离线自校验。
- **Sentinel HASP V3 engine 启用高级保护**: 开启 "Periodic .text CRC checks" (如果 V3 engine 支持),或从应用层自检: Editor 解密后主动 SHA256 关键 licensing 函数页,比对编译期 baked hash。
- **IAT hardening**: 对不在 KnownDLLs 的导入 DLL 在启动时做 `WinVerifyTrust` + name-hash 校验,阻断 DLL hijack。
- **ULF 签名升级**: SHA-1 → SHA-256/EdDSA,全文签名覆盖完整 ULF 文档。

### 长期

- **多层纵深**: license 判定 = Sentinel 保护 + 应用层自检 + 云端心跳三重验证,任一层缺失不予放行。
- **TPM/VBS backing**: 将 license 根密钥与 TPM/Windows VBS 绑定,确保管理员权限也不可提取或修改。
- **ASAR 强制校验 + Hub 逻辑 native 化**: 将 license 相关的关键判定逻辑从 Electron JS 迁移到 native 模块。

---

## 漏洞披露原则与状态

本研究在企业内部研究环境进行,以内部资产风险评估为目的。PoC 仅供内部使用,未向 Tuanjie 或 Thales 提交。

- Tuanjie 官网: https://unity.cn/tuanjie
- Thales Sentinel LDK 安全联系: https://cpl.thalesgroup.com/software-monetization/sentinel-ldk

## 参考

外部文档:

- Sentinel Envelope — Functionality (Thales): https://docs.sentinel.thalesgroup.com/ldk/LDKdocs/SPNL/LDK_SLnP_Guide/Protection/Functionality_Envelope.htm
- Disabling Protection Keys When Tampering Is Detected: https://docs.sentinel.thalesgroup.com/ldk/LDKdocs/SPNL/LDK_SLnP_Guide/Protection/Windows/Disable_HL_key.htm
- Periodic Background Check Behavior: https://docs.sentinel.thalesgroup.com/softwareandservices/ldk/LDKdocs/Env-Win/Envelope/Periodic_background_check_behavior.htm
- HASP SL — A Deeper Dig (Tuts4You community): https://forum.tuts4you.com/files/file/1335-hasp-sl-a-deeper-dig/
- ASAR format specification: https://github.com/electron/asar
- W3C XML Signature Syntax and Processing Version 1.1
- Microsoft PE / PDB / DLL Forwarder documentation (MSDN)
