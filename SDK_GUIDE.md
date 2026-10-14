# pdfxml SDK 详细说明

> 当前仓库源码的 XFDF/Link/Caret 与高级注释属性升级尚未发布。新诊断 API 与公开类型迁移见第 9 节，已发布的 `0.1.1` 不包含这些新增能力。

这份文档是给“第一次接触这个项目的人”准备的。

你可以把 `pdfxml` 理解成两种用法：

1. **命令行工具**：直接在终端里运行
2. **Rust SDK / library**：在你自己的 Rust 项目里当库来调用

---

## 1. 这个项目到底在做什么
[out.pdf](examples/out.pdf)
很多 PDF 标注工具可以把注释单独导出成 `XFDF` 文件。
cargo run -p pdfxml-cli -- --from-pdf -i examples/out.pdf -o exported.xfdf

cargo run -p pdfxml-cli -- -i exported.xfdf -t examples/WTchbOFP.pdf -o examples/out2.pdf -v
比如：
- 高亮
- 便签
- 删除线
- 线条
- 矩形
- 圆形
- 多边形

这些信息本身不是 PDF 页面内容，而是“注释数据”。

`pdfxml` 现在支持两条方向：

- **XFDF -> PDF**
- **PDF -> XFDF**

所以它像一个“双向翻译器”：

- 输入 XFDF，输出 PDF
- 或输入 PDF，导出标准 XFDF

---

## 2. CLI 和 SDK 的区别

### CLI 用法

适合你只想直接转换文件：

```bash
pdfxml -i annotations.xfdf -o output.pdf
pdfxml --from-pdf -i annotated.pdf -o exported.xfdf
```

如果你是从仓库源码直接运行 CLI，则使用：

```bash
cargo run -p pdfxml-cli -- --from-pdf -i examples/out.pdf -o exported.xfdf
cargo run -p pdfxml-cli -- -i exported.xfdf -t examples/WTchbOFP.pdf -o examples/out2.pdf -v
```

### SDK 用法

适合你想在自己的 Rust 程序里调用：

```rust
use pdfxml::{load_xfdf, export_annotations};
```

如果你是直接从 Git 仓库接入库，可以在自己的 `Cargo.toml` 里这样写：

```toml
[dependencies]
pdfxml = { git = "https://github.com/hearsay316/pdfXml.git" }
```

如果后续发布到 crates.io，再改成版本依赖即可。

比如你的系统里已经有：
- 文件上传流程
- 批量处理任务
- Web API
- 桌面软件

那你就不一定想再绕一层命令行，而是想在代码里直接调用库函数。

---

## 3. 项目结构怎么理解

现在项目大致可以这样理解：

```text
Cargo.toml           # 根库 crate + workspace 入口
cli/
├── Cargo.toml       # CLI crate 配置
└── src/
    └── main.rs      # 命令行入口（很薄，只负责接参数和调用库）
src/
├── lib.rs           # 对外暴露的 SDK 入口
├── xfdf.rs          # 解析 XFDF/XML，并可重新序列化为 XFDF
├── pdf.rs           # 生成 PDF / 从 PDF 读取注释
├── annotation.rs    # 注释的数据结构定义
└── error.rs         # 错误类型
```

### `src/lib.rs`

这是库的门面。

外部项目通常不需要知道内部每个文件怎么组织，直接从这里导入就行：

```rust
use pdfxml::{XfdfDocument, PdfAnnotationExporter};
```

### `cli/src/main.rs`

这是命令行入口。

它自己不做复杂业务，只做：

1. 读取参数
2. 调用库 API
3. 打印结果

### `src/xfdf.rs`

负责两件事：

1. 把 XFDF/XML 文本解析成 Rust 结构体
2. 把内部注释模型重新写回标准 XFDF 字符串

### `src/pdf.rs`

负责两件事：

1. 把注释真正写进 PDF
2. 从 PDF 页面的 `/Annots` 里读回注释

### `src/annotation.rs`

定义各种注释长什么样。

比如：
- Text
- Highlight
- Square
- Circle
- Line
- Polygon

### `src/error.rs`

统一定义错误类型。

这样不管是：
- 读文件失败
- XML 解析失败
- PDF 处理失败

最后都能用比较统一的方式返回。

---

## 4. 最常用的公开 API

目前最重要的入口主要有这几个。

### 4.1 `XfdfDocument::parse`

**把一段 XFDF/XML 字符串解析成 `XfdfDocument`。**

```rust
use pdfxml::XfdfDocument;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xfdf = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text page="0" rect="100,700,250,730" color="#FFFF00">
      Hello World
    </text>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf)?;
    println!("注释数量: {}", doc.annotations.len());
    Ok(())
}
```

### 4.2 `load_xfdf`

**直接从文件读取并解析 XFDF。**

```rust
use pdfxml::load_xfdf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    println!("注释数量: {}", doc.annotations.len());
    Ok(())
}
```

### 4.3 `load_annotations_from_pdf`

**从 PDF 读取注释，并转换成统一的 `XfdfDocument`。**

```rust
use pdfxml::load_annotations_from_pdf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_annotations_from_pdf("annotated.pdf")?;
    println!("注释数量: {}", doc.annotations.len());
    Ok(())
}
```

### 4.4 `XfdfDocument::to_xfdf_string`

**把内部注释模型重新序列化成标准 XFDF。**

```rust
use pdfxml::load_annotations_from_pdf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_annotations_from_pdf("annotated.pdf")?;
    let xml = doc.to_xfdf_string()?;
    println!("{}", xml);
    Ok(())
}
```

### 4.5 `export_pdf_annotations_to_xfdf`

**把 PDF 里的注释直接写成 XFDF 文件。**

```rust
use pdfxml::export_pdf_annotations_to_xfdf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    export_pdf_annotations_to_xfdf("annotated.pdf", "annotated.xfdf")?;
    Ok(())
}
```

### 4.6 `PdfAnnotationExporter::new`

**创建一个导出器。**

```rust
use pdfxml::PdfAnnotationExporter;

let mut exporter = PdfAnnotationExporter::new();
```

### 4.7 `export_to_new_pdf`

**新建一个 PDF，把注释写进去。**

```rust
use pdfxml::{PdfAnnotationExporter, XfdfDocument};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let xfdf = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text page="0" rect="100,700,250,730" color="#FFFF00">
      Hello World
    </text>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf)?;
    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&doc, Path::new("output.pdf"))?;
    Ok(())
}
```

### 4.8 `export_annotations`

**这是一个更省事的顶层包装函数。**

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    export_annotations(&doc, Some("original.pdf"), "annotated.pdf")?;
    Ok(())
}
```

---

## 5. 一个完整的 round-trip 思路

如果你想验证“写出去的 PDF，能不能再读回 XFDF”，可以走这个流程：

1. `load_xfdf(...)`
2. `export_annotations(...)` 生成 PDF
3. `load_annotations_from_pdf(...)`
4. `to_xfdf_string()` 或 `export_pdf_annotations_to_xfdf(...)`

```rust
use pdfxml::{export_annotations, load_annotations_from_pdf, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let src = load_xfdf("annotations.xfdf")?;
    export_annotations(&src, None::<&str>, "roundtrip.pdf")?;

    let loaded = load_annotations_from_pdf("roundtrip.pdf")?;
    let xml = loaded.to_xfdf_string()?;

    std::fs::write("roundtrip.xfdf", xml)?;
    Ok(())
}
```

---

## 6. 当前 PDF -> XFDF 的边界

第一版重点是：

- 读取 `/Annots`
- 识别当前项目已支持的常见注释类型
- 转回现有 `Annotation` / `XfdfDocument`
- 输出标准 XFDF

当前策略是：

- 已支持的类型尽量完整导出
- 不支持的 `/Subtype` 跳过并记录 warning
- 某些 PDF 字段不能 100% 还原成原始 XFDF 时，以“可用、可回放”为优先

比如：

- `Stamp` 第一版不会伪造原始 `imagedata`
- `Popup` 会尽量找父注释名字
- `PolyLine` 会导出成 `<polyline>` 而不是 `<polygon>`

---

## 7. 如果你要继续扩展，先看哪里

推荐阅读顺序：

1. `src/lib.rs`
2. `src/annotation.rs`
3. `src/xfdf.rs`
4. `src/pdf.rs`
5. `tests/integration_test.rs`

如果你要扩展 **XFDF -> PDF**：优先看 `src/pdf.rs` 的 `build_annotation(...)`

如果你要扩展 **PDF -> XFDF**：优先看 `src/pdf.rs` 的 `load_annotations_from_pdf(...)` 和 `annotation_from_pdf_dict(...)`

如果你要扩展 **序列化**：优先看 `src/xfdf.rs` 的 `to_xfdf_string()`

---

## 8. 总结

现在这个库已经不只是“把 XFDF 写成 PDF”。

它已经具备：

- 解析 XFDF
- 导出 PDF
- 从 PDF 读回注释
- 再导出成标准 XFDF

所以后面不管你要做：

- SDK 发布
- 批量转换
- round-trip 校验
- 更完整的 PDF 注释兼容

都已经有一个比较清晰的基础。

---

## 9. XFDF 升级：诊断、交互注释、高级属性与迁移

### 9.1 读取并检查诊断

```rust
use pdfxml::{export_annotations, load_xfdf_with_options, XfdfParseOptions};

fn main() -> pdfxml::Result<()> {
    let report = load_xfdf_with_options("examples/interactive.xfdf", &XfdfParseOptions::default())?;
    for diagnostic in &report.diagnostics {
        eprintln!("{:?} {:?} <{}> byte {}: {}",
            diagnostic.severity, diagnostic.code, diagnostic.element,
            diagnostic.byte_offset, diagnostic.message);
    }
    for warning in report.document.pdf_conversion_warnings() {
        eprintln!("PDF 转换限制：{warning}");
    }
    export_annotations(&report.document, None::<&str>, "interactive.pdf")?;
    Ok(())
}
```

已有字符串时使用 `XfdfDocument::parse_with_report(xml)`。
原有 `parse` / `load_xfdf` 仍可用，但只返回文档并通过 `log` 输出警告；需要可靠展示警告时应使用 report API。

### 9.2 严格模式与输入上限

```rust
use pdfxml::{XfdfDocument, XfdfParseMode, XfdfParseOptions};

fn main() -> pdfxml::Result<()> {
    let options = XfdfParseOptions {
        mode: XfdfParseMode::Strict,
        max_input_bytes: 8 * 1024 * 1024,
        max_annotations: 10_000,
        max_page_index: 1_000,
        ..Default::default()
    };
    let report = XfdfDocument::parse_with_options(
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots><caret page="0" rect="0,0,20,20"/></annots></xfdf>"#,
        &options,
    )?;
    assert_eq!(report.document.annotations.len(), 1);
    Ok(())
}
```

严格模式遇到 Warning 时返回错误；保留扩展的 Info 诊断不会失败。
它不是完整 XFDF XSD 校验，更不是“全部内容可无损写入 PDF”的开关。`fields`、扩展 XML、富文本样式、命名目的地等限制仍需检查 `pdf_conversion_warnings()`。
输入必须是 UTF-8/XML 1.0，允许 UTF-8 BOM；DTD、外部实体和不完整 XML 无论何种模式都不接受。
序列化会再次按默认 XML 资源边界检查结果；手工构造的大文档不能绕过该检查。

### 9.3 Link / Caret 模型

- `Annotation::Link(LinkAnnotation)`：`target` 是 `Option<LinkTarget>`，避免同时存在多个跳转目标。
- `LinkTarget::Uri { uri, is_map }`：只存储地址，不发起请求。
- `LinkTarget::Named { name }`：仅名称引用，不复制或生成名称树。
- `LinkTarget::Page { page, view }`：`page` 是零基索引；`view` 使用 `LinkDestinationMode` 和有序的 `Vec<Option<f64>>`。参数顺序可查 `parameter_names()`；`None` 表示 PDF `null`。
- `LinkHighlightMode`：`None`、`Invert`、`Outline`、`Push`，与目标页面显示模式互相独立。
- `Annotation::Caret(CaretAnnotation)`：`CaretSymbol::None` / `P`，`fringe` 对应 PDF `/RD` 四边差值。

新 PDF 页数仍由注释实际所在页决定。内部 Link 指向不存在的页会在保存前失败；不会写出一个假页面引用，也不会仅根据链接目标自动创建大量空白页。

### 9.4 公开类型迁移（有源码兼容性变化）

`XfdfDocument` 新增 `file_reference`、`ids`、`unsupported_annotations`、`extensions`，`XfdfField` 新增 `additional_values`。
以前直接列出所有字段的结构体字面量需要补上新字段，建议使用默认值：

```rust
use pdfxml::{XfdfDocument, XfdfField};

let document = XfdfDocument {
    fields: vec![XfdfField {
        name: "choices".into(),
        value: Some("first".into()),
        additional_values: vec!["second".into()],
        ..Default::default()
    }],
    ..Default::default()
};
assert_eq!(document.fields[0].values().collect::<Vec<_>>(), ["first", "second"]);
```

- `value` 仍表示第一个值；`additional_values` 为后续值；`values()` 按顺序遍历全部值。无值与 `Some("")` 区分。
- 穷举匹配 `Annotation` 的代码需补充 `Link`、`Caret` 分支。
- `AnnotationBase` 字面量需补充 `intent: None`、`border_style: None`、`dashes: None`；该类型没有 `Default` 实现，仍需显式初始化其余字段。
- `FreeTextAnnotation` 新增 `callout: Option<CalloutLine>`、`fringe: Option<Rect>`、`head_style: LineEndingStyle`、`width: f32`；字面量须补齐。普通旧文本可设为 `None`、`None`、`LineEndingStyle::None`、`0.0`。
- `PolygonAnnotation` 新增 `width: f32`，旧字面量通常补 `width: 1.0`。这些新增字段提供 Serde 默认值，但不能据此认为 Rust 结构体字面量仍源码兼容。
- FreeText 的 `callout`、`fringe`、`width`、`head` 和通用 `intent` 不再留在 `extra`；有效边框 `style/dashes` 也转为类型化字段。依赖旧扩展字典取值的调用方需改用公开字段。
- `annotation_type()` 对未闭合多边形现在返回 `PolyLine` 而非 `Polygon`，类型统计/筛选代码需同步。
- 文本输出统一为 `<contents>`，旧式标签直接文本仍可读；不要依赖旧的 XML 字面格式或属性排序。
- 命名 flags 优先输出；历史 `flags="10"` 仍按 **十六进制 0x10** 解释，不改成十进制 10。推荐使用 `print,readonly` 或显式 `0x...`。
- 兼容模式会记录无效已建模属性的降级；未实现注释放入 `unsupported_annotations`，不再默默当成已支持注释。
- 未建模注释属性和子元素的 XFDF 保留不等于 PDF 映射；FreeText Callout/Fringe 已打通字典和 AP，仍未映射的扩展应检查 `pdf_conversion_warnings()`。
- XFDF 的旧 `IT` / `border-style` 输出规范化为 `intent` / `style`；PDF 回读可能明确输出原来隐含的实线边框，不能依赖 XML 字面完全相同。

本批不直接发布或修改包版本。由于以上公开类型变化，后续发版应按破坏性升级处理，不能作为 `0.1.x` 的普通补丁发布。

### 9.5 可运行示例与边界

仓库示例：

- `examples/interactive.xfdf`：URI 链接、第二页跳转和两种 Caret。
- `examples/advanced-annotations.xfdf`：7 条高级属性示例，包含两种 Callout、虚线/细线宽形状和仅保留元数据的测量 Intent。

```bash
cargo run -p pdfxml-cli -- --strict -i examples/advanced-annotations.xfdf -o advanced.pdf
cargo run -p pdfxml-cli -- --from-pdf -i advanced.pdf -o advanced-roundtrip.xfdf
cargo run -p pdfxml-cli -- --strict -i advanced-roundtrip.xfdf -o advanced-roundtrip.pdf
```

支持清单见 `ANNOTATION_SUPPORT.md`，实施记录见 `XFDF_UPGRADE_PROGRESS.md`。
`FileAttachment`、`Sound`、`Widget/AcroForm`、完整富文本样式和安全删密均未在本批实现。

### 9.6 Intent、FreeText Callout 与 Border/Dash

以下示例使用第二批新增的类型化 API：

```rust
use pdfxml::{
    Annotation, AnnotationBase, BorderStyle, CalloutLine, FreeTextAnnotation,
    LineEndingStyle, Rect, XfdfDocument,
};

let annotation = FreeTextAnnotation {
    base: AnnotationBase {
        name: Some("callout-knee".into()),
        page: 0,
        rect: Some(Rect { left: 40.0, bottom: 560.0, right: 550.0, top: 760.0 }),
        title: None,
        subject: None,
        contents: Some("Callout with a knee".into()),
        creation_date: None,
        modification_date: None,
        color: None,
        opacity: 0.8,
        flags: 0,
        intent: Some("FreeTextCallout".into()),
        border_style: Some(BorderStyle::Dashed),
        dashes: Some(vec![6.0, 3.0]),
        extra: Default::default(),
    },
    default_style: None,
    default_appearance: Some("/Helvetica 14 Tf".into()),
    text_color: Some("#1A4A80".into()),
    align: 0,
    callout: Some(CalloutLine::from_coordinates(&[70.0, 590.0, 200.0, 650.0, 300.0, 685.0]).unwrap()),
    fringe: Some(Rect { left: 260.0, bottom: 100.0, right: 20.0, top: 20.0 }),
    head_style: LineEndingStyle::ClosedArrow,
    width: 2.0,
};
let document = XfdfDocument {
    annotations: vec![Annotation::FreeText(annotation)],
    ..Default::default()
};
assert!(document.pdf_conversion_warnings().is_empty());
```

- `intent` 是 PDF 名称而非行为枚举，不能为空或包含空白/控制字符。`LineDimension`、`PolygonCloud` 等名称可往返，但目前不执行测量/云线绘制，转换会提示限制。
- `CalloutLine::from_string("x1,y1,x2,y2")` / `from_coordinates` 接受 4 或 6 个有限数；`start` 是标记端点，`knee` 是可选折点，`end` 是文字框连接点。`coordinates()` 可还原有序坐标。
- `fringe` 复用 `Rect` 的字段表示四边差值，**不是绝对矩形**；必须非负且留下有效文字区域。
- 普通 XFDF FreeText 未给宽度时为 0；带 Callout 时推断为 1。手工构造模型的 `width` 不会自动变成 1；`width=0` 不绘制线/端点，仍保留文字及几何。
- `BorderStyle` 五种样式可读写 `/BS`。Solid/Dashed 有显式 AP；Bevelled/Inset/Underline 目前仅写字典，显示依赖阅读器且会报告转换限制。
- 虚线数组允许空值；非空数组要求非负有限数且至少有一个正值。未指定样式但有数组时推断 Dashed；Dashed 无数组采用 `[3]`；显式 Solid 不使用数组绘制。
- Callout 伸出原边界时导出会扩大 PDF Rect/BBox 并重算 RD，以保护端点和文字框；因此首次 PDF 回读的 Rect/Fringe 可能不同，但文字框位置不变，原 SDK 对象不被修改。
- 这些字段支持新 PDF 和合并已有 PDF；导出时校验手工构造值，失败不会覆盖已有输出。类型化字段应作为唯一数据来源，不要在 `extra` 中再塞同名属性。
