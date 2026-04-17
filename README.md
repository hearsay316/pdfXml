# pdfxml

[English](#english) | [中文](#中文)

---

## 中文

`pdfxml` 是一个 **Rust 库 crate**，用于在 XFDF/XML 与 PDF 注释之间进行转换。

注意：
- `pdfxml` **只提供库能力，不提供 CLI 可执行程序**
- 命令行工具请安装 **`pdfxml-cli`**

### 功能概览

- 解析 XFDF/XML 注释
- 将 XFDF/XML 注释导出到新 PDF
- 将 XFDF/XML 注释合并到现有 PDF
- 从 PDF 中提取注释并导出为 XFDF
- 支持常见注释类型：Text、Highlight、Underline、StrikeOut、FreeText、Square、Circle、Line、Polygon、Ink、Stamp、Popup

### 安装

#### 作为库使用

```toml
[dependencies]
pdfxml = "0.1.1"
```

#### 安装 CLI

```bash
cargo install pdfxml-cli
```

### 库示例

#### XFDF -> PDF

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    export_annotations(&doc, Some("original.pdf"), "annotated.pdf")?;
    Ok(())
}
```

#### PDF -> XFDF

```rust
use pdfxml::{export_pdf_annotations_to_xfdf, load_annotations_from_pdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_annotations_from_pdf("annotated.pdf")?;
    println!("注释数量: {}", doc.annotations.len());
    export_pdf_annotations_to_xfdf("annotated.pdf", "annotated.xfdf")?;
    Ok(())
}
```

### 文档

- API 文档（docs.rs）：<https://docs.rs/pdfxml>
- SDK 文档：`SDK_GUIDE.md`
- 注释支持范围：`ANNOTATION_SUPPORT.md`
- CLI 文档：`cli/README.md`

### 开发与测试

```bash
cargo test --workspace
```

### 中文 FreeText 字体

当导出包含中文的 FreeText 注释时，程序会尝试从系统中寻找可用 CJK 字体。
如需显式指定字体，可设置环境变量：

```bash
PDFXML_CJK_FONT=/path/to/font.ttf
```

### 许可证

本项目采用 MIT 许可证。详情见仓库中的 `LICENSE` 文件。

### 致谢

- Adobe XFDF 规范
- `lopdf`、`quick-xml`、`ttf-parser` 等 Rust 生态项目
- Rust 社区

---

## English

`pdfxml` is a **Rust library crate** for converting between XFDF/XML and PDF annotations.

Important:
- `pdfxml` provides **library APIs only; it does not ship a CLI binary**
- For command-line usage, install **`pdfxml-cli`**

### Features

- Parse XFDF/XML annotations
- Export XFDF/XML annotations into a new PDF
- Merge XFDF/XML annotations into an existing PDF
- Extract annotations from PDF and export them as XFDF
- Supports common annotation types: Text, Highlight, Underline, StrikeOut, FreeText, Square, Circle, Line, Polygon, Ink, Stamp, Popup

### Installation

#### Use as a library

```toml
[dependencies]
pdfxml = "0.1.1"
```

#### Install the CLI

```bash
cargo install pdfxml-cli
```

### Library examples

#### XFDF -> PDF

```rust
use pdfxml::{export_annotations, load_xfdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_xfdf("annotations.xfdf")?;
    export_annotations(&doc, Some("original.pdf"), "annotated.pdf")?;
    Ok(())
}
```

#### PDF -> XFDF

```rust
use pdfxml::{export_pdf_annotations_to_xfdf, load_annotations_from_pdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let doc = load_annotations_from_pdf("annotated.pdf")?;
    println!("annotation count: {}", doc.annotations.len());
    export_pdf_annotations_to_xfdf("annotated.pdf", "annotated.xfdf")?;
    Ok(())
}
```

### Documentation

- API docs (docs.rs): <https://docs.rs/pdfxml>
- SDK guide: `SDK_GUIDE.md`
- Annotation support matrix: `ANNOTATION_SUPPORT.md`
- CLI guide: `cli/README.md`

### Development

```bash
cargo test --workspace
```

### CJK FreeText fonts

When exporting Chinese or other CJK FreeText annotations, the crate will try to locate a usable system font.
You can also specify one explicitly:

```bash
PDFXML_CJK_FONT=/path/to/font.ttf
```

### License

This project is licensed under the MIT License. See the `LICENSE` file in the repository for details.

### Acknowledgements

- Adobe XFDF specification
- Rust ecosystem projects such as `lopdf`, `quick-xml`, and `ttf-parser`
- The Rust community
