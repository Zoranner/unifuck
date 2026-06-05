/// 顶层 vendor 区分: Unity 与 Tuanjie 是两家独立公司,
/// 顶层一切按 vendor 分家,底层工具 (ASAR/PE/RSA/hosts) 保持 vendor 无关。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vendor {
    Unity,
    Tuanjie,
}

impl Vendor {
    /// UI 与日志中的简短显示名 ("Unity" / "Tuanjie")。
    pub fn display_name(self) -> &'static str {
        match self {
            Vendor::Unity => "Unity",
            Vendor::Tuanjie => "Tuanjie",
        }
    }

    /// Program Files 下的 Hub 安装目录名 ("Unity Hub" / "Tuanjie Hub")。
    pub fn hub_name(self) -> &'static str {
        match self {
            Vendor::Unity => "Unity Hub",
            Vendor::Tuanjie => "Tuanjie Hub",
        }
    }

    /// %APPDATA% 下 Hub 配置目录名 ("UnityHub" / "TuanjieHub")。
    pub fn config_dir_name(self) -> &'static str {
        match self {
            Vendor::Unity => "UnityHub",
            Vendor::Tuanjie => "TuanjieHub",
        }
    }

    /// Hub 全局共享 Licensing Client 目录名。
    pub fn licensing_client_dir(self) -> &'static str {
        match self {
            Vendor::Unity => "UnityLicensingClient_V1",
            Vendor::Tuanjie => "TuanjieLicensingClient_V1",
        }
    }

    /// Editor Licensing DLL 文件名 (无目录)。
    pub fn dll_name(self) -> &'static str {
        match self {
            Vendor::Unity => "Unity.Licensing.EntitlementResolver.dll",
            Vendor::Tuanjie => "Tuanjie.Licensing.EntitlementResolver.dll",
        }
    }

    /// Editor 主程序文件名。
    pub fn exe_name(self) -> &'static str {
        match self {
            Vendor::Unity => "Unity.exe",
            Vendor::Tuanjie => "Tuanjie.exe",
        }
    }

    /// Program Files 下独立 Editor 目录前缀 ("Unity " / "Tuanjie ")。
    pub fn editor_dir_prefix(self) -> &'static str {
        match self {
            Vendor::Unity => "Unity ",
            Vendor::Tuanjie => "Tuanjie ",
        }
    }

    /// 用于排序的稳定顺序: Unity 在前, Tuanjie 在后。
    pub fn sort_order(self) -> u8 {
        match self {
            Vendor::Unity => 0,
            Vendor::Tuanjie => 1,
        }
    }

    /// 迭代所有 vendor,用于发现循环。
    pub fn all() -> &'static [Vendor] {
        &[Vendor::Unity, Vendor::Tuanjie]
    }

    /// Editor 读取 license 文件的目录 (`C:\ProgramData\<Vendor>`)。
    pub fn license_dir(self) -> &'static str {
        match self {
            Vendor::Unity => r"C:\ProgramData\Unity",
            Vendor::Tuanjie => r"C:\ProgramData\Tuanjie",
        }
    }

    /// License 文件名 (`<Vendor>_lic.ulf`)。
    pub fn license_filename(self) -> &'static str {
        match self {
            Vendor::Unity => "Unity_lic.ulf",
            Vendor::Tuanjie => "Tuanjie_lic.ulf",
        }
    }

    /// 签 license 用的 RSA 私钥 PEM (从 keys/<vendor>/mod/priv.key 嵌入)。
    pub fn priv_key_pem(self) -> &'static str {
        match self {
            Vendor::Unity => include_str!("../keys/mod/priv.key"),
            Vendor::Tuanjie => include_str!("../keys/tuanjie/mod/priv.key"),
        }
    }

    /// 是否需要 native runtime hook (DLL forwarder loader)。Tuanjie Editor
    /// 2022.3+ 的 IPC AuthorizationCheck 由 Sentinel HASP envelope 守护,
    /// 必须在内存里 patch `IsValidIPCResponse`,纯 cert + ULF 替换不够。
    pub fn supports_native_hook(self) -> bool {
        matches!(self, Vendor::Tuanjie)
    }
}
