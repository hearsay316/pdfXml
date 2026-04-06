# pdfxml SDK 详细说明

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
