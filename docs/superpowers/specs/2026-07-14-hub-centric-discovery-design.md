# Hub 为中心的安装发现模型

## 动机

当前 `find_hub_roots()` 只搜注册表 + `<drive>:\Program Files\<HubName>`，自定义路径的 Hub 完全漏掉。连锁影响三处：Editor 发现、共享 Licensing Client 发现、Hub ASAR 列表。

现有一维列表把 Hub 和 Editor 平铺，缺少归组关系。用户直觉是"选 Hub → 自动处理其下所有 Editor"。

## 数据模型

```
InstallGroup
├─ hub: HubInfo
├─ editors: Vec<EditorInstall>
├─ shared_client_patched: bool
├─ selected: bool
└─ expanded: bool
```

所有 Editor 必须归属某个 Hub。不存在无 Hub 的独立 Editor。

### 分组规则

| 来源 | 归属组 |
|------|--------|
| 厂商默认托管目录 `<前缀>\<Vendor>\Hub\Editor\<version>` | 该厂商 Hub 组 |
| editors.json 中记录的路径 | 该 Hub 组 |

去重：按可解析的真实安装路径去重；路径不存在时按 Windows 大小写不敏感的规范化文本去重。同一 Editor 只出现在一个组内。

## 发现流程

```
scan_groups():
  1. 收集所有 Hub 根目录（注册表 + Program Files + config.custom_hub_paths）
  2. 对每个 Hub 根：
     a. 验证 resources\app.asar → 构建 HubInfo
     b. 读编辑器配置 (editors-v2.json / editors.json) → 验证 Editor 可执行文件
     c. 扫厂商默认托管目录；前缀来自 secondaryInstallPath.json、ProgramFiles 和 C:\Program Files 的并集 → 去重合并
     d. 检查共享 Licensing Client 的 .dll.bak → shared_client_patched
     e. 产出 InstallGroup
  3. 排序：按 vendor (Unity 在前) → Hub 版本降序
```

干掉原来的阶段1（Program Files 独立扫描）。没有 Hub 的 Editor 不显示；用户需要先配 Hub 路径。

## 持久化

`%APPDATA%\unifuck\config.json`:

```json
{
  "custom_hub_paths": {
    "Unity Hub": "D:\\Tools\\Unity Hub"
  }
}
```

不修改 Hub 的 `editors.json`。

## UI

### 列表呈现

```
[Unity Hub 3.11.0]                   PATCHED     ← 组头，可展开/折叠
    2022.3.50f1   D:\...\Editor\      UNPATCHED   ← 缩进子项
    6000.1.25f1   D:\...\Editor\      UNPATCHED
    Licensing Client                   UNPATCHED   ← 共享客户端状态行
[Tuanjie Hub 1.2.3]                  UNPATCHED
    2022.3.50f1   D:\...\Editor\      UNPATCHED
```

空状态显示 `[ ADD HUB FOLDER ]` 按钮。

### 选择语义

| 操作 | 效果 |
|------|------|
| 选中组头 | 切换整组（Editor + 共享 Client） |
| 取消组头 | 取消整组 |
| 选中/取消子 Editor | 独立切换，与组头解耦 |
| 选中/取消共享 Client 行 | 只影响共享 Client |
| 混合状态 | 组头显示部分选中样式 |
| PATCH/UNPATCH 模式 | 跟现在一致：同组内选中项必须 patched 状态一致 |

### 操作入口

- ActionBar 加 `+` 按钮 → 弹出原生文件夹选择器选 Hub 目录
- 空状态区域加 `[ ADD HUB FOLDER ]` 按钮
- 使用 `IFileOpenDialog` + `FOS_PICKFOLDERS`（复用现有 `windows-sys`）

## 涉及文件

| 文件 | 改动 |
|------|------|
| `discovery.rs` | `find_hub_roots()` 新增 config 来源；config 读写函数 |
| `core/group.rs` (新) | 替换 `install.rs`；`scan_groups()` → `Vec<InstallGroup>` |
| `core/mod.rs` | 导出新类型 |
| `gui/app.rs` | `AppState` 改用 `Vec<InstallGroup>`；`add_custom_hub_path()` |
| `gui/bindings.rs` | 选择逻辑适配分组；绑定新 callback |
| `gui/jobs.rs` | `collect_targets()` 适配分组模型 |
| `app-window.slint` | 列表支持组头/子项层次；`+` 按钮 |
| `action-bar.slint` | 加 `+` 按钮 |
| `main.rs` | 文件对话框 + 新 callback 连接 |

## 窗口尺寸

列表变深后空间偏紧，`min-height: 320` → `400`。

## 兼容性

- config.json 不存在时行为不变（只搜注册表 + Program Files）
- 现有 patch/unpatch 操作逻辑不变，只是目标选择从平铺改为分组
- editors.json 读取逻辑保留，归属到对应 Hub 组
