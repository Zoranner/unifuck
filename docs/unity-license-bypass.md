# Unity License System 安全研究报告

## 摘要

研究对象: Unity Hub 3.x (Electron ASAR 应用) 与 Unity Editor 2021-2022 (Mono/.NET 混合应用) 的 license 验证系统在离线部署场景下的实施强度。报告梳理了系统架构、识别了多个层面的实施弱点,涵盖 Hub 登录态判定、Editor 公钥嵌入、ULF XML 数字签名、Shared Licensing Client IPC、域名屏蔽等环节。研究以内部资产风险评估为目的展开,对每个弱点给出技术细节、利用方法与缓解建议。整体评估: 离线场景下 license 验证依赖本地状态,所有信任锚点 (公钥、JS 判定函数、域名解析) 均可被本地管理员级修改,系统未实施篡改检测或在线状态回溯。

| 维度 | 评级 | 说明 |
|------|------|------|
| 信息泄漏 | 低 | 不涉及远程信息泄漏,风险面限于本地 |
| 完整性 | 高 | ULF 签名仅覆盖 Terms 子树,公钥以 base64 文本嵌入 .NET 程序集,可被定向字节替换绕过 |
| 所需权限 | 管理员 | 写入 Program Files / ProgramData 需要管理员权限 |

## 影响范围

| 组件 | 典型路径 | 格式 | 版本范围 |
|------|---------|------|---------|
| Unity Hub | `C:\Program Files\Unity Hub` | Electron ASAR | 3.x |
| Shared Licensing Client | `<Unity Hub>\UnityLicensingClient_V1\` | .NET DLL | 与 Editor 共用 |
| Unity Editor | `C:\Program Files\Unity <version>\Editor\` | Native + .NET | 2021、2022 系列 |
| ULF License File | `C:\ProgramData\Unity\Unity_lic.ulf` | XML DSig | 与 Editor 匹配 |
| 云端 License Server | `license.unity3d.com` / `api.unity.com` | HTTPS | 在线验证 |

## 系统架构

Unity license 系统组件关系:

```
Unity Hub (Electron, ASAR)
    ├─ AuthService.js       用户登录状态管理
    ├─ licenseService.js    本地 license 缓存与校验
    ├─ licensingSdk.js      对接 Licensing Client IPC
    └─ [Named Pipe IPC] → UnityLicensingClient_V1 (.NET 后台进程)
           └─ Unity.Licensing.EntitlementResolver.dll
                ├─ ULF XML Schema 验证 + XMLDSig 签名校验
                ├─ 嵌入式 X.509 公钥 (RSA 4096)
                └─ 结果 → Hub → 下发给 Editor

Unity Editor (Unity.exe)
    ├─ 2021 单体: 内嵌 LicensingLegacy 模块,含第 2 份相同公钥
    ├─ 2022+: 独立 Licensing Client 进程
    └─ ULF 读取: Editor 启动时从 C:\ProgramData\Unity\ 读 .ulf

云端 license.unity3d.com
    ├─ Hub 在线登录 → 在线签发并下发 ULF
    └─ ULF 由 Unity 签发私钥签名 (RSA-SHA1, W3C XML DSig)
```

数据流: 用户 → Hub 登录 → 调云端 API → 云端返回 ULF → Hub 存 `C:\ProgramData\Unity\Unity_lic.ulf` → Editor 启动 → 读 ulf → EntitlementResolver 验签名 → 返回 granted/denied → Editor 按 license 运行。

## 弱点一: Hub ASAR 无强制完整性校验

### 机制

Unity Hub 基于 Electron 框架,应用逻辑 (JavaScript/HTML/CSS) 压缩为 ASAR 归档文件。ASAR 格式在 header JSON 中包含每个文件的 `size` 和 `offset`,顶层含 SHA256 哈希字段用于完整性校验。但 Electron 在默认 build 配置下不强制校验此哈希,且未通过 Electron Fuses 烧写 `EnableEmbeddedAsarIntegrityValidation`。

### 利用

解包 ASAR 归档,修改目标 JS 文件,重新打包并更新 header hash 后,Hub 正常启动,不报告任何校验失败。操作流程:

1. 解析 ASAR header JSON,获取文件列表与布局
2. 按 offset/size 提取目标 JS 文件内容
3. 修改 JS 函数体 (见弱点二)
4. 将修改后的内容写回 ASAR,更新 header 中的 size/offset,重新计算 SHA256

### 影响

JS 是 Hub 授权逻辑的唯一载体。ASAR 完整性不可强制执行意味着任意 JS 逻辑可被静默替换,Editor 无法感知 Hub 端判定的完整性。

**严重度: 高。**

---

## 弱点二: Hub 登录态与 license 判定全本地,无服务端回溯

### 机制

Hub 中的授权逻辑分布在多个 JS 文件中:

- `AuthService.js`: 管理用户登录状态 (`isUserSignedIn`, `restoreAuthState`, `constructor`)
- `licenseService.js`: 缓存并校验本地 license 文件 (`isLicenseValid`, `getValidEntitlementGroups`)
- `licensingSdk.js`: 对接后台 .NET Licensing Client 的 IPC 桥 (`init`, `getInstance`)

这些函数运行在本地 Electron 进程内,离线状态下完全跳过服务端同步。云端仅在网络可达时做异步 token 刷新;离线时函数直接访问本地内存状态。

### 利用

通过替换 JS 函数体,可令 Hub 在完全断网状态下呈现"已登录 + 有效 Pro license":

**`licenseService.js`: license 判定替换**

```js
// 原始: isLicenseValid 读本地文件 + 调用 SDK 校验
// 替换为:
isLicenseValid() {
    return __awaiter(this, void 0, void 0, function* () {
        return true;
    });
}

// 原始: getValidEntitlementGroups 从 SDK 取有效 entitlement 列表
// 替换为注入伪造的 Pro Max Ultra 组:
getValidEntitlementGroups() {
    return __awaiter(this, void 0, void 0, function* () {
        return [{
            startDate: new Date('1970-01-01'),
            expirationDate: new Date('9999-01-01'),
            productName: 'Pro',
            licenseType: 'ULF',
            entitlementGroupId: 'pro-001',
            validationErrors: [],
        }];
    });
}
```

**`AuthService.js`: 登录态伪造**

```js
// 构造函数中注入伪造的 userInfo,令 isUserSignedIn 判定通过:
constructor() {
    super();
    this.connectInfo = {
        error: false, initialized: true, loggedIn: true,
        maintenance: false, online: false, ready: true,
        showLoginWindow: false, workOffline: true
    };
    this.userInfo = {
        foreignKey: 'local-fk',
        name: 'Offline User',
        email: 'offline@local',
        primaryOrg: 'local',
        createdAt: '1970-01-01T00:00:00.000Z',
    };
}

// isUserSignedIn 改用本地注入的 userInfo:
isUserSignedIn() {
    return true;
}

// restoreAuthState 直接设置 loggedIn 而不走云端 token 校验:
restoreAuthState() {
    return __awaiter(this, void 0, void 0, function* () {
        this.connectInfo.ready = true;
        this.connectInfo.loggedIn = true;
        this.connectInfo.initialized = true;
        this.connectInfo.workOffline = true;
        // 广播登录事件,驱动 UI 更新
        this.broadcast('CONNECT_INFO_CHANGED', this.connectInfo);
        this.broadcast('USER_INFO_CHANGED', { userInfo: this.userInfo });
        this.broadcast('LOGGED_IN');
    });
}
```

**`licensingSdk.js`: IPC 桥替换**

```js
// init 跳过网络校验:
init(correlationId) {
    return __awaiter(this, void 0, void 0, function* () {
        return true;
    });
}

// getInstance 返回 stub,避免对真实 LicensingClient 的调用返回 null 导致 NPE:
getInstance() {
    return {
        getConnectedPipeName() { return ''; },
        connect() { return Promise.resolve(false); },
        dispose() { return Promise.resolve(); },
        getMachineId() { return Promise.resolve(''); },
        getAllEntitlementGroups() {
            return Promise.resolve({ entitlementGroupsDetails: [] });
        },
        checkEntitlement() { return Promise.resolve(true); },
        // ...其余方法同样给安全默认值
    };
}
```

### 格式差异

Unity Hub 存在两种 JS 打包格式:

- CommonJS (Hub 3.5/3.7): 独立 service 文件,使用 `__awaiter` 异步包装器
- ESM (Hub 3.18+): 单文件 `app-*.js` bundle,使用 native `async/await` 和 private fields (`#field`)

两种格式受同一类弱点影响,只是具体函数体替换的语法稍有差异 (CommonJS 需要 `__awaiter` 包装,ESM 用 `async function` 直接返回)。

### 影响

Hub 是用户获取 license 的入口,而其登录/授权状态完全由本地 JS 决定,无服务端逆向验证。在完全隔离内网的机器上,上述改动可使 Hub 以"已登录 + Pro license"状态运行,且 Editor 继承此状态。

**严重度: 高。**

---

## 弱点三: ULF 签名验证公钥以 .NET 资源嵌入,可被一致性替换

### 机制

ULF (Unity License File) 是 W3C XML DSig 格式的签名文件。Editor 使用 `Unity.Licensing.EntitlementResolver.dll` (一个 .NET 程序集) 校验 ULF 签名。该校验使用的 X.509 公钥 (RSA 4096) 以其 DER 编码或 PEM base64 字符串字面量形式存储在 DLL 的 .NET #US (User String) 堆中。DLL 本身无 Authenticode 签名校验,无运行时代码完整性保护。

在 Unity 2021 及之前版本,公钥的 base64 表示还同时嵌入 Editor 主程序 `Unity.exe` (~73 MB) 内的 LicensingLegacy 模块中。2022+ 中 `Unity.exe` 是一个约 282 KB 的启动器,不含证书。

### 利用

用自签的 RSA 4096 密钥对 (生成 `e=65537`,模长 4096 bit,PKCS#1 编码约 550 字节) 替换 DLL 及 exe 中的原始公钥:

**步骤:**

1. 生成自签 RSA 4096 密钥对,提取公钥的 DER 编码
2. 将原始公钥 DER 和自签公钥 DER 分别做 base64 编码,按 76 字符一行切行 (标准 PEM 格式)
3. 在 DLL 二进制中以原始 base64 行为搜索模板,原地替换为自签 base64 的对应行
4. 自签与原始公钥均为 4096-bit RSA,衍生的 DER 字节数相同,base64 行数相同,替换长度严格对齐,不改变 PE header、.NET metadata 或 IL 的任何偏移量
5. 对 `Unity.exe` (若体积 >1MB,即为 2021 单体版本) 做同样搜索-替换 — 其中证书可能以单行 (不换行) 形态嵌入,也需匹配单行情形

伪代码:

```
original_der = extract_der(original_cert_pem)
modified_der = extract_der(modified_cert_pem)
assert len(original_der) == len(modified_der)

b64_orig_lines = base64_encode_lines(original_der, line_width=76)
b64_mod_lines  = base64_encode_lines(modified_der, line_width=76)

for each line pair (orig_line, mod_line) in zip(b64_orig_lines, b64_mod_lines):
    find all occurrences of orig_line bytes in dll_bytes
    replace in-place with mod_line bytes

// 同时处理单行形态 (某版本 exe 中 base64 在一个连续字符串内)
single_orig = concatenate_all_lines(b64_orig_lines)
single_mod  = concatenate_all_lines(b64_mod_lines)
if single_orig found in dll_bytes in single-line form:
    replace in-place
```

### Shared Licensing Client 副本

Unity Hub 的 `<Hub>\UnityLicensingClient_V1\` 目录下有一份相同的 `Unity.Licensing.EntitlementResolver.dll`,Editor 通过 IPC 连接此副本进行签名校验。若仅替换 Editor 本地 DLL 而未替换此副本,实际生效的仍是原始公钥。需要同步替换。

### 影响

公钥是 ULF 签名信任链的唯一锚点。替换公钥等同于破坏整个信任模型——攻击者用自己的私钥签名 ULF,替换后的公钥将验签通过。

**严重度: 高。**

---

## 弱点四: ULF 签名使用 SHA-1,仅覆盖 Terms 子树

### 机制

Unity ULF 采用 W3C XML DSig 规范签名:

- 签名算法: RSA PKCS#1 v1.5, 摘要算法 SHA-1
- 规范化方法: Inclusive Canonicalization with Comments (C14N)
- 签名范围: 仅覆盖 `<License id="Terms">` 子树,非整个 ULF 文档
- ULF 结构包含多个区域: Subject, License (Terms), Signature 等;除 Terms 子树外的区域不在签名覆盖范围内

SHA-1 已于 2015 年被 NIST 淘汰 (chosen-prefix collision 可行,复杂度约 2^60)。ULF 中仅 Terms 子树被签名意味着 unsigned 区域的元素 (非 License 区域) 可被增删而不影响签名验证。

### 利用

自签 ULF 流程:

1. 准备 ULF 模板 XML,含 `<License id="Terms">` 子树内的静态授权信息 (ProductName, Features, Validity 等),以及空占位 `<DigestValue/>` 和 `<SignatureValue/>`
2. 对 `<License id="Terms">` 子树做 C14N 规范化 → SHA-1 hash → base64 填入 `<DigestValue>`
3. 对 `<SignedInfo>` 区域 (含 Reference URI 指向 `#Terms`) 做 C14N 规范化 → 用自签 RSA 私钥做 PKCS#1 v1.5 SHA-1 签名 → base64 填入 `<SignatureValue>`
4. 将生成的 ULF XML 部署到 `C:\ProgramData\Unity\Unity_lic.ulf`

伪代码:

```
template_xml = load_ulf_template()
c14n_license = canonicalize_subtree(template_xml, xpath="//License[@id='Terms']")
digest = SHA1(c14n_license)
template_xml.replace("<DigestValue/>", "<DigestValue>" + base64(digest) + "</DigestValue>")

c14n_signed_info = canonicalize_signed_info(template_xml)
signature = RSA_PKCS1v15_Sign(private_key, SHA1, c14n_signed_info)
template_xml.replace("<SignatureValue/>", "<SignatureValue>" + base64(signature) + "</SignatureValue>")

deploy(template_xml, "C:\\ProgramData\\Unity\\Unity_lic.ulf")
```

验证: 自签 ULF 中的 `<DigestValue>` 经独立 lxml C14N 确认与部署版 bit-identical;签名字段经 `cryptography` 库 PKCS#1 v1.5 RSA-SHA1 验签通过。patched DLL 内的自签公钥对自签 ULF 数学验签成功。

### 影响

SHA-1 的碰撞风险和子树签名的覆盖不足组合,降低了 ULF 签名的有效性。主要威胁上限: 需要先突破公钥存储 (弱点三) 才能利用签名弱点;签名弱点本身不能单独突破,但减少了后续攻击的难度。

**严重度: 中。**

---

## 弱点五: 域名屏蔽可阻断在线激活与心跳

### 机制

Unity Hub 和 Editor 在启动时向以下域名发起 HTTPS 请求:

- `license.unity3d.com` — 在线 license 签发与验证
- `core.cloud.unity3d.com` — 用户身份与组织信息
- `api.unity.com` — 云端 API 聚合

这些域名通过标准 DNS 解析,解析结果受 Windows hosts 文件控制 (`C:\Windows\System32\drivers\etc\hosts`)。当域名被解析到 `127.0.0.1`,连接被拒绝;Hub / Editor 走离线降级路径,信任本地已存在的 ULF 文件与内嵌公钥。未发现 Hub 实施"离线超过 N 天则拒绝"的宽限期逻辑。

### 利用

向 hosts 文件追加:

```
127.0.0.1 license.unity3d.com
127.0.0.1 core.cloud.unity3d.com
127.0.0.1 api.unity.com
```

操作要点:

- 以标记块形式写入,包含起止标识 (`# begin-block` / `# end-block`),确保可干净移除
- 写入前做当前内容备份 (`.bak`),防误写
- 写入时检查是否已有同标记块,避免重复追加

### 影响

阻断域名使云端无法线上拉黑或撤销 license,且无心跳超时自动失效机制。此弱点不直接破坏加密体系,但关闭了远程管控的最后一层网关。

**严重度: 中。**

---

## 利用链路

将所有弱点串联,完整 bypass 路径:

| 步骤 | 动作 | 依赖弱点 |
|------|------|---------|
| 1 | 生成自签 RSA 4096 密钥对 | — |
| 2 | 替换 Editor DLL 与 exe 中的 ULF 公钥 (base64 逐行替换) | 弱点三 |
| 3 | 同步替换 Hub Shared Licensing Client 的同名 DLL | 弱点三 |
| 4 | 用自签 RSA 私钥按 XML DSig 签名一份 ULF | 弱点四 |
| 5 | 部署 ULF 到 `C:\ProgramData\Unity\Unity_lic.ulf` | 弱点四 |
| 6 | 解包 Hub ASAR,修改 licenseService/AuthService/licensingSdk 函数体,重打包 | 弱点一、弱点二 |
| 7 | hosts 屏蔽云端域名 | 弱点五 |
| 8 | 启动 Hub → Hub 离线模式,本地 JS 判定已登录 → Editor 读 ULF → patched DLL 验签通过 → 激活 | — |

回滚: 步骤 2/3/4/6 均在对目标文件修改前创建 `.bak` 副本;步骤 5 删除 ULF 文件;步骤 7 从 hosts 移除标记块。所有操作可逆。

**需要权限:** 所有写操作目标 (`Program Files`, `ProgramData`, `drivers\etc\hosts`) 均需要管理员权限。

---

## 缓解建议

### 短期

- **ASAR 完整性强制**: 使用 `electron-builder` 的 afterPack hook 或 post-pack 脚本验证 ASAR header hash;或启用 Electron Fuses 的 `EnableEmbeddedAsarIntegrityValidation`。
- **域名解析加固**: 对关键 license 域使用 DoH (DNS over HTTPS) 解析或证书公钥 pinning,不依赖系统 hosts 文件。
- **ULF 签名升级**: RSA-SHA1 → RSA-SHA256 或 ECDSA (P-256)。

### 中期

- **公钥保护**: 将公钥从 .NET 字符串字面量改为加密资源,密钥派生自 machine SID + DPAPI,运行时解密,用完擦除内存。对 DLL 增加 Authenticode 自校验 (`WinVerifyTrust`),阻断不可检测的 DLL 修改。
- **ULF 全文签名**: 签名范围从 `<License id="Terms">` 子树扩展到整个文档。
- **IPC 双向认证**: Editor ↔ Licensing Client 的命名管道通信增加双向 challenge-response 与进程 token 校验,阻断中间人。
- **离线宽限期**: Editor 即便离线也应设定最长宽限期 (如 30 天),到期降级为只读模式,直到重新在线同步。

### 长期

- **本地状态降格为缓存**,最终判定需云端或离线时间窗共同约束。license 视为短期租赁而非永久所有,周期在线续租。
- **Hub 逻辑 native 化**: 将 license 相关的关键判定从 Electron JS 迁移到 native 模块 (C++/Rust),加编译期混淆与 self-hash 校验,缩小 JS 攻击面。
- **TPM / VBS enclave**: 高敏感场景将 license seeding 数据存入 TPM 或 VBS,管理员权限亦不可修改。

---

## 漏洞披露原则与状态

本研究在企业内部研究环境进行,以内部资产风险评估为目的。PoC 仅供内部使用,未向 Unity Technologies 提交。若需要走 responsible disclosure:

- Unity 安全联系: security@unity3d.com
- Unity Bug Bounty: https://hackerone.com/unity

## 参考

- W3C XML Signature Syntax and Processing Version 1.1 (W3C Recommendation)
- ASAR format specification: https://github.com/electron/asar
- Electron fuses: https://www.electronjs.org/docs/latest/tutorial/fuses
- Microsoft Authenticode / WinVerifyTrust: MSDN PE Certificate Table documentation
- NIST SP 800-131A (SHA-1 deprecation guidance)
