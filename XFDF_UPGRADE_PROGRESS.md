# XFDF 升级实施记录（第一、二批）

更新：2026-10-14

状态：**第一批 XFDF 基础保真/诊断与 Link、Caret 已落地；第二批 Intent、FreeText Callout/Fringe 和 Border/Dash 已落地，整个升级方案尚未全部完成。**

## 1. 本批范围与版本说明

第一批优先修复数据丢失和静默降级问题，再打通两种交互注释的 XFDF→PDF→XFDF 通路。
第二批建立高级属性的类型化模型、PDF 字典读写和支持范围内的 AP；补足数值验证与未实现外观的转换警告。
没有实施 FileAttachment、Widget/AcroForm、完整富文本排版或通用性能重构。

仓库根库和 CLI 当前清单版本为 `0.1.1`，GUI 为 `0.1.0`；本批未修改包版本、发布 crate 或创建 Git 提交。
原方案中的目标版本、完成度百分比、阅读器兼容性百分比不作为本批验收依据。
新公开结构体字段及枚举分支会影响源码兼容性，后续应按破坏性版本升级发布，而不是以 `0.1.x` 补丁隐式替换。

## 2. 已完成

### 2.1 第一批：XFDF 解析与往返

- 使用有资源边界的 XML 树解析；验证根节点、命名空间、属性名称与重复属性，拒绝畸形/未闭合 XML。
- 正确解码属性实体，拼接文本/CDATA，保留 `<contents>` 首尾空白与显式空值。
- 正确区分 `f/href` 文档引用和 `fields`；支持 `ids`、根属性、嵌套字段、多值字段。
- 兼容历史 `f/fields` 包装并报告规范化行为，不再把 `f` 误当成字段。
- 嵌套 Popup 不再覆盖父注释，支持继承页码并建立父子关联。
- 保存富文本 XML、未建模的注释属性/子元素和文档扩展；未实现注释独立保留并警告。
- 属性消费规则按注释类型区分，避免不适用字段吞掉其他类型的扩展；第二批将 FreeText 的 `fringe/callout` 正式提升为类型化字段。
- 保留从祖先继承的扩展属性命名空间；不将伪造命名空间的同名标签认成已支持类型。
- 支持命名 flags，保留历史无前缀十六进制解释，输出使用稳定属性顺序和未刻意截断的数值。
- 修复非 ASCII 颜色字符串可能导致的切片 panic，拒绝非有限或无法写入 PDF 数值范围的矩形坐标。

### 2.2 第一批：诊断与调用入口

- 新增 `XfdfParseOptions`、`XfdfParseReport` 和结构化诊断类型。
- 提供 `parse_with_report`、`parse_with_options`、`load_xfdf_with_options`。
- 默认兼容模式保留可处理数据并报告问题，严格模式遇到 Warning 即失败。
- 提供 `pdf_conversion_warnings()`，明确表单、扩展、富文本和命名目的地的转换限制。
- CLI 新增 `--strict`，不能与 `--from-pdf` 同用；版本输出改为读取清单版本。
- GUI 的 XFDF→PDF 结果消息附带解析警告及转换限制。
- 有界文件读取先检查字节大小，再检查 UTF-8，不把截断多字节字符误报成编码问题。

### 2.3 第一批：Link

- 新增目标、页面显示方式和点击高亮模式的类型化模型。
- XFDF 支持 URI、Named、页面目的地，以及 GoTo 包装；简写输入规范化为标准子元素。
- PDF 读写 URI、`/Dest`、`/A /GoTo`、`/H`、边框宽度和 QuadPoints。
- 覆盖 8 种页面显示模式及允许的 `null` 参数。
- 页面树构建完成后回填真实对象引用；支持向合并后新增页面跳转。
- 目标页不存在时保存前失败，既有输出不被覆盖，导出器可继续复用。
- 不执行链接，也不实现脚本、Launch 或远程动作。

### 2.4 第一批：Caret 与富文本保留

- Caret 支持 `None` / `P`、`fringe` 与 PDF `/Sy`、`/RD`。
- 为有效可绘制范围生成插入符或段落符 AP，支持颜色和透明度。
- 富文本以 PDF `/RC` 保存和回读；FreeText 可视 AP 仍采用纯文本，未宣称实现样式排版。
- `PolyLine` 类型名称不再误报为 `Polygon`。

### 2.5 第二批：Intent、FreeText Callout/Fringe 与 Border/Dash

- `AnnotationBase` 新增 `intent`、`border_style`、`dashes`；新增 `BorderStyle`、`LineEndingStyle`、`CalloutLine`，FreeText 和 Polygon 补齐线宽等字段。
- XFDF 标准 `intent` 与旧 `IT` 均可输入，输出规范化；PDF `/IT` 往返，不再把所有 FreeText 强制写成 `FreeTextTypeWriter`。
- FreeText 的 4/6 数字引出线、端点样式和四边差值对应 `/CL`、单名称 `/LE`、`/RD`；无显式意图时推断 Callout。
- Callout AP 组合文字 Form、引出线、端点、文字框、虚线与透明度；西文复用字体文字 AP，中文复用系统字体字形轮廓。
- 引出线越出原矩形时扩大 `/Rect` 和 `/BBox` 并重算 `/RD`，避免箭头被截掉或文字框整体移动/缩放；不修改调用方模型。
- 支持五种 `/BS /S` 样式和 `/D` 虚线数组。实线/虚线生成 AP；bevelled/inset/underline 不假冒成实线 AP，而是仅保留字典并明确提示依赖阅读器。
- FreeText、Square、Circle、Line、Polygon/PolyLine 的 AP 应用虚线；Ink、Link 保留 BS 并交给阅读器呈现。PDF 回读兼容间接虚线数值和旧 `/Border` 数组。
- 保留 0.25 等细线宽，修复细矩形边框被 AP 边界裁切；显式 0 不描边、不退化为 hairline。Line/Callout 补足 Diamond、Butt、反向箭头和 Slash 路径。
- 非法宽度、坐标、Fringe、虚线、Intent 与对齐值会验证；畸形 PDF 数字数组不再静默丢弃非数字项来凑出合法坐标。
- 已支持的高级属性可导出新 PDF 和合并已有 PDF；新增回归验证原有注释和页面内容保留，以及失败时不覆盖已有输出。
- 测量/云线/高亮型墨迹等 Intent 只保存元数据，未知/CSS 边框样式仅作为 XFDF 扩展保留；严格解析并不把这些能力变成已实现。

## 3. 主要文件

| 文件 | 用途 |
| --- | --- |
| `src/annotation.rs` | Link/Caret、高级属性模型与基础数值校验 |
| `src/xfdf.rs`、`src/xfdf/parser.rs` | 解析、诊断、扩展保留、序列化 |
| `src/pdf.rs`、`src/pdf/interactive.rs` | PDF 字典、页面引用、Caret AP、RC、图形/箭头 AP |
| `src/pdf/advanced.rs` | 高级字段验证、BS 读写、虚线/透明度、Callout 组合 AP |
| `src/lib.rs` | SDK 导出与有界文件读取 |
| `cli/src/main.rs`、`gui/src/service.rs` | 调用端警告与 CLI 严格开关 |
| `tests/xfdf_upgrade_test.rs` | 第一批 31 项回归测试 |
| `tests/advanced_annotation_test.rs` | 第二批 18 项高级属性回归测试 |
| `examples/interactive.xfdf` | 2 个 Link 和 2 个 Caret 的两页示例 |
| `examples/advanced-annotations.xfdf` | 7 条 Callout、虚线、细线宽及 Intent 示例 |
| `ANNOTATION_SUPPORT.md`、`SDK_GUIDE.md` | 支持边界及公开 API 迁移 |

## 4. 验证结果

### 自动化验证

| 范围 | 结果 |
| --- | --- |
| 库单元测试 | 15 通过 |
| 集成测试（含 PDF 输入兼容） | 15 通过（原有 7 项 + 新增 8 项） |
| 新增 XFDF/交互注释回归 | 31 通过 |
| 新增高级属性回归 | 18 通过 |
| CLI 参数测试 | 2 通过 |
| Rust 文档测试 | 15 通过 |
| SDK_GUIDE 第 9.6 节示例 | 从 Markdown 提取，独立编译并执行通过 |
| GUI 服务层独立测试 | 4 通过（原有 2 项 + 独立 harness 的 2 项转换验证） |
| CLI 编译 | 通过 |
| Clippy | 通过，仍有 3 条既有警告：文档列表缩进 1 条、`chunks_exact` 建议 2 条 |

合计 **85 项单元/集成测试与 15 项 Rust 文档测试通过**，包含第二批之后新增的 8 项 PDF 输入兼容回归。Clippy 不含本批新增代码警告。

回归覆盖仓库真实 XFDF 样本、实体/CDATA/空值、嵌套字段和 Popup、扩展命名空间、未知注释、资源上限、DTD、非法坐标、全部目的地模式、URI/GoTo、页对象引用、合并补页、Caret AP、RC，以及失败时不覆盖输出。
第二批额外覆盖 Intent 别名/冲突、Callout 4/6 坐标、文字资源与空内容、中文轮廓/缺字体错误、边界扩展、10 种端点、五种边框样式、默认/空/实线虚线数组、细线宽与零宽、间接数值及畸形 PDF 数组、高级属性合并。
中文回归在无系统字体的环境验证明确缺字体错误；本机已使用 `simsun.ttc` 成功生成中文轮廓，另做了实际渲染验证。

### 实际命令行验证

在 `target/xfdf-smoke/` 中生成验证文件，没有覆盖仓库示例或用户文件：

1. 严格读取 `examples/interactive.xfdf` → 两页 PDF。
2. PDF → XFDF：保留 **2 个 Link + 2 个 Caret**。
3. 回读 XFDF 再严格转换为 PDF 成功。
4. 含 FileAttachment 的输入：兼容模式明确警告并只输出支持的 Caret。
5. 同一输入使用严格模式失败，预先写入的输出文件保持不变。

第二批使用重新编译的 CLI 验证：

1. `examples/advanced-annotations.xfdf` 严格导出 `advanced.pdf`，PDF→XFDF→PDF 三步成功，**7 条注释全部保留**。
2. `LineDimension` 保留为 `/IT`，CLI 明确显示不执行测量的转换警告。
3. 临时中文 Callout 样本同样完成严格往返，使用 `C:/Windows/Fonts/simsun.ttc`，中文内容与引出线保留。

### 实际渲染验证（MuPDF 单引擎）

- 在 Git 忽略的 `target/core-check/pdf-render-tools/` 局部安装 PyMuPDF 1.28.2，不改项目依赖和全局 Python 环境。
- 以 1.5 倍比例渲染高级属性示例和其往返 PDF：均有 7 条注释，PNG 为 893 × 1263，前后像素完全一致。
- 目视检查确认直线/折线引出线、开口/闭合箭头、文字框、虚线矩形/椭圆/线/多边形及透明度可见。
- 中文 Callout 两份 PDF 渲染同样像素一致，并检查文字框裁剪图确认“中文引出线”可见。
- PNG 与临时中文样本保存在 `target/xfdf-smoke/`，不是源码交付物；**这不是跨阅读器一致性认证**。

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

### 真实 PDF 输入兼容补丁（2026-10-14）

用户要求包直接兼容原文件，而不是修改 PDF 后再调用。新增共享加载入口 `src/pdf/reader.rs`，同时接入 PDF→XFDF 与合并 API，不修改依赖版本，也不需要外部 PDF 工具作为运行时依赖。

- 根因复现：原文件的表头为 `xref 0 175`；底层解析器要求 `xref` 后换行。最初怀疑 `WebFastLoad` 尾部数据，后用标准 xref + 同样尾部数据的回归样本排除了这一判断。
- 处理范围：原样加载失败后，只根据最新 `startxref` 定位并在内存中适配单个空格/制表符，字节长度与对象偏移不变。不是扫描替换所有 `xref`，也不回退到旧修订。
- 新增 8 项回归：直接读取、合并并保留原批注和正文、标准 xref + 尾部数据、非法偏移不覆盖输出、损坏最新修订不回退、内容流字节保持、前置字节、有效增量修订读取最新值。
- 直接使用原始 `examples/cc.xml` 与 `examples/mwnodbDo.pdf` 生成 `examples/mwnodbDo-cc-merged.pdf`：23 页，327 条主批注 + 11 个 Popup，共 338 个对象，327 个命名标识全部存在。
- 类型统计：Highlight 68、FreeText 44、Line 14、Underline 53、Squiggly 51、Ink 87、Stamp 2、Square 5、Text 3、Popup 11。
- 两个输入的 SHA-256 均与操作前相同。23 页原始内容流逐字节一致；在仅用于验证的内存副本中排除新增批注后，正文提取和 1 倍 MuPDF 渲染均与原文件一致，未保存该副本。
- 渲染抽查第 1、2、13、14、17、18 页：中文、高亮、手写和图章可显示；44 个 FreeText 中 43 个非空文本有 AP，另 1 个原本为空。长文本、富文本和高亮型 Ink 的专用外观仍按现有限制处理，不能称为原标注软件的无损复刻。
- 输出 PDF 另存，不改输入；第三方 XML 扩展/富文本排版/专用 Intent 的转换限制仍明确记录在日志。

验证统计与渲染截图位于 Git 忽略的 `target/xfdf-smoke/cc-*`。这是本机样本验收，不是跨阅读器或通用损坏 PDF 修复能力的认证。

## 5. 使用与迁移

在正常构建环境可直接运行：

```bash
cargo run -p pdfxml-cli -- --strict -i examples/interactive.xfdf -o interactive.pdf
cargo run -p pdfxml-cli -- --from-pdf -i interactive.pdf -o interactive-roundtrip.xfdf
cargo run -p pdfxml-cli -- --strict -i examples/advanced-annotations.xfdf -o advanced.pdf
cargo run -p pdfxml-cli -- --from-pdf -i advanced.pdf -o advanced-roundtrip.xfdf
cargo run -p pdfxml-cli -- --strict -i advanced-roundtrip.xfdf -o advanced-roundtrip.pdf
```

SDK 的旧函数保留，但以下源码需迁移，详见 `SDK_GUIDE.md` 第 9 节：

- `XfdfDocument` / `XfdfField` 字面量补充新字段或 `..Default::default()`。
- 穷举 `Annotation` 的匹配补充 Link/Caret。
- `AnnotationBase`、`FreeTextAnnotation`、`PolygonAnnotation` 字面量补新字段；原来从 `extra` 读取 Intent/Callout/Fringe/Border 的代码改为类型化字段。
- `value` 仍是字段首值，使用 `values()` 读取包括多选值在内的全部值。
- 依赖 `annotation_type()`、XML 字面输出或无前缀 flags 的调用方检查行为说明。

## 6. 下一批顺序与验收要求

1. **FileAttachment**：建立 Filespec/EmbeddedFile 模型，明确附件大小上限、文件名净化、导入/导出与路径策略；禁止隐式读写任意本地路径。
2. **审阅关系**：Reply/State、命名标识、Popup 关联校验与失败诊断。
3. **表单专项**：Widget/AcroForm 字段树、控件绑定、值填写与外观更新；不能仅把 `<fields>` 写入注释列表。
4. **高级外观补全**：测量/云线 Intent、云状边框、bevelled/inset/underline 显式 AP 等，独立定义验收样本。
5. **富文本与阅读器验收**：逐步覆盖样式子集，建立跨阅读器样本/截图对比；单独评估性能。
6. **发布前**：补齐完整 workspace/桌面 GUI 验证、公开 API 版本升级和发布清单。

严格模式不等于无损转换，原始 XML 保留不等于 PDF 支持；URI 也没有经过安全净化。完整限制以 `ANNOTATION_SUPPORT.md` 为准。
