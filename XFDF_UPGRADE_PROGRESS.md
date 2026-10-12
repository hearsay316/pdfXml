# XFDF 升级实施记录（第一批）

日期：2026-10-09  
状态：**XFDF 基础保真/诊断与 Link、Caret 已落地，整个升级方案尚未全部完成。**

## 1. 本批范围与版本说明

本批优先修复数据丢失和静默降级问题，再打通两种交互注释的 XFDF→PDF→XFDF 通路。
没有实施 FileAttachment、Widget/AcroForm、完整富文本排版或通用性能重构。

仓库根库和 CLI 当前清单版本为 `0.1.1`，GUI 为 `0.1.0`；本批未修改包版本、发布 crate 或创建 Git 提交。
原方案中的目标版本、完成度百分比、阅读器兼容性百分比不作为本批验收依据。
新公开结构体字段及枚举分支会影响源码兼容性，后续应按破坏性版本升级发布，而不是以 `0.1.x` 补丁隐式替换。

## 2. 已完成

### 2.1 XFDF 解析与往返

- 使用有资源边界的 XML 树解析；验证根节点、命名空间、属性名称与重复属性，拒绝畸形/未闭合 XML。
- 正确解码属性实体，拼接文本/CDATA，保留 `<contents>` 首尾空白与显式空值。
- 正确区分 `f/href` 文档引用和 `fields`；支持 `ids`、根属性、嵌套字段、多值字段。
- 兼容历史 `f/fields` 包装并报告规范化行为，不再把 `f` 误当成字段。
- 嵌套 Popup 不再覆盖父注释，支持继承页码并建立父子关联。
- 保存富文本 XML、未建模的注释属性/子元素和文档扩展；未实现注释独立保留并警告。
- 属性消费规则按注释类型区分，避免 Caret 的 `fringe` 等新字段导致 FreeText 同名扩展丢失。
- 保留从祖先继承的扩展属性命名空间；不将伪造命名空间的同名标签认成已支持类型。
- 支持命名 flags，保留历史无前缀十六进制解释，输出使用稳定属性顺序和未刻意截断的数值。
- 修复非 ASCII 颜色字符串可能导致的切片 panic，拒绝非有限或无法写入 PDF 数值范围的矩形坐标。

### 2.2 诊断与调用入口

- 新增 `XfdfParseOptions`、`XfdfParseReport` 和结构化诊断类型。
- 提供 `parse_with_report`、`parse_with_options`、`load_xfdf_with_options`。
- 默认兼容模式保留可处理数据并报告问题，严格模式遇到 Warning 即失败。
- 提供 `pdf_conversion_warnings()`，明确表单、扩展、富文本和命名目的地的转换限制。
- CLI 新增 `--strict`，不能与 `--from-pdf` 同用；版本输出改为读取清单版本。
- GUI 的 XFDF→PDF 结果消息附带解析警告及转换限制。
- 有界文件读取先检查字节大小，再检查 UTF-8，不把截断多字节字符误报成编码问题。

### 2.3 Link

- 新增目标、页面显示方式和点击高亮模式的类型化模型。
- XFDF 支持 URI、Named、页面目的地，以及 GoTo 包装；简写输入规范化为标准子元素。
- PDF 读写 URI、`/Dest`、`/A /GoTo`、`/H`、边框宽度和 QuadPoints。
- 覆盖 8 种页面显示模式及允许的 `null` 参数。
- 页面树构建完成后回填真实对象引用；支持向合并后新增页面跳转。
- 目标页不存在时保存前失败，既有输出不被覆盖，导出器可继续复用。
- 不执行链接，也不实现脚本、Launch 或远程动作。

### 2.4 Caret 与富文本保留

- Caret 支持 `None` / `P`、`fringe` 与 PDF `/Sy`、`/RD`。
- 为有效可绘制范围生成插入符或段落符 AP，支持颜色和透明度。
- 富文本以 PDF `/RC` 保存和回读；FreeText 可视 AP 仍采用纯文本，未宣称实现样式排版。
- `PolyLine` 类型名称不再误报为 `Polygon`。

## 3. 主要文件

| 文件 | 用途 |
| --- | --- |
| `src/annotation.rs` | Link/Caret 模型与基础数值校验 |
| `src/xfdf.rs`、`src/xfdf/parser.rs` | 解析、诊断、扩展保留、序列化 |
| `src/pdf.rs`、`src/pdf/interactive.rs` | PDF 字典、页面引用、Caret AP、RC |
| `src/lib.rs` | SDK 导出与有界文件读取 |
| `cli/src/main.rs`、`gui/src/service.rs` | 调用端警告与 CLI 严格开关 |
| `tests/xfdf_upgrade_test.rs` | 本批新增 31 项回归测试 |
| `examples/interactive.xfdf` | 2 个 Link 和 2 个 Caret 的两页示例 |
| `ANNOTATION_SUPPORT.md`、`SDK_GUIDE.md` | 支持边界及公开 API 迁移 |

## 4. 验证结果

### 自动化验证

| 范围 | 结果 |
| --- | --- |
| 库单元测试 | 15 通过 |
| 原有集成测试 | 7 通过 |
| 新增 XFDF/交互注释回归 | 31 通过 |
| CLI 参数测试 | 2 通过 |
| Rust 文档测试 | 15 通过 |
| GUI 服务层独立测试 | 4 通过（原有 2 项 + 独立 harness 的 2 项转换验证） |
| CLI 编译 | 通过 |
| Clippy | 通过，仍有 3 条既有警告：文档列表缩进 1 条、`chunks_exact` 建议 2 条 |

回归覆盖仓库真实 XFDF 样本、实体/CDATA/空值、嵌套字段和 Popup、扩展命名空间、未知注释、资源上限、DTD、非法坐标、全部目的地模式、URI/GoTo、页对象引用、合并补页、Caret AP、RC，以及失败时不覆盖输出。

### 实际命令行验证

在 `target/xfdf-smoke/` 中生成验证文件，没有覆盖仓库示例或用户文件：

1. 严格读取 `examples/interactive.xfdf` → 两页 PDF。
2. PDF → XFDF：保留 **2 个 Link + 2 个 Caret**。
3. 回读 XFDF 再严格转换为 PDF 成功。
4. 含 FileAttachment 的输入：兼容模式明确警告并只输出支持的 Caret。
5. 同一输入使用严格模式失败，预先写入的输出文件保持不变。

### 验证环境边界

完整 workspace 的离线构建因 GPUI 的 Git 依赖未缓存而失败；**没有完成桌面 GUI 编译/交互验收**。
此外，仓库 `.cargo/config.toml` 指向本机不存在的 MSVC 14.50 链接器，本次仅通过环境变量选择已安装的 14.44，没有改动该配置。

因此使用被 Git 忽略的 `target/core-check/` 独立清单，直接指向真实库、CLI、测试和 GUI 服务源文件，不引入 GPUI：

```powershell
$env:CARGO_HOME = 'D:\code2026\pdfXml\target\core-check\cargo-home'
$env:CARGO_TARGET_DIR = 'D:\code2026\pdfXml\target\xfdf-check-build'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'C:\Program Files\Microsoft Visual Studio\18\Insiders\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64\link.exe'
cargo test --manifest-path target/core-check/Cargo.toml --offline --workspace
cargo clippy --manifest-path target/core-check/Cargo.toml --offline --workspace --all-targets
```

该清单/GUI harness 是本机验证产物，不是交付依赖或仓库正式构建方式。
具备 GUI 依赖和有效编译工具链的环境仍应执行 `cargo test --workspace`。
本批没有进行跨阅读器视觉对比、性能基准或完整 XFDF 规范一致性认证。

## 5. 使用与迁移

在正常构建环境可直接运行：

```bash
cargo run -p pdfxml-cli -- --strict -i examples/interactive.xfdf -o interactive.pdf
cargo run -p pdfxml-cli -- --from-pdf -i interactive.pdf -o interactive-roundtrip.xfdf
```

SDK 的旧函数保留，但以下源码需迁移，详见 `SDK_GUIDE.md` 第 9 节：

- `XfdfDocument` / `XfdfField` 字面量补充新字段或 `..Default::default()`。
- 穷举 `Annotation` 的匹配补充 Link/Caret。
- `value` 仍是字段首值，使用 `values()` 读取包括多选值在内的全部值。
- 依赖 `annotation_type()`、XML 字面输出或无前缀 flags 的调用方检查行为说明。

## 6. 下一批顺序与验收要求

1. **高级属性**：Intent、FreeText Callout/Fringe、Border/Dash；同时实现 XFDF 字段、PDF 字典和 AP，不能只保存字符串就标记“已支持”。
2. **FileAttachment**：建立 Filespec/EmbeddedFile 模型，明确附件大小上限、文件名净化、导入/导出与路径策略；禁止隐式读写任意本地路径。
3. **审阅关系**：Reply/State、命名标识、Popup 关联校验与失败诊断。
4. **表单专项**：Widget/AcroForm 字段树、控件绑定、值填写与外观更新；不能仅把 `<fields>` 写入注释列表。
5. **富文本与阅读器验收**：逐步覆盖样式子集，建立跨阅读器样本/截图对比；单独评估性能。
6. **发布前**：补齐完整 workspace/桌面 GUI 验证、公开 API 版本升级和发布清单。

严格模式不等于无损转换，原始 XML 保留不等于 PDF 支持；URI 也没有经过安全净化。完整限制以 `ANNOTATION_SUPPORT.md` 为准。
