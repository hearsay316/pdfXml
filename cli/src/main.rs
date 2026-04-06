//! 这是命令行程序的入口文件。
//!
//! 它自己不做复杂的 PDF 或 XML 处理，
//! 主要负责三件事：
//! 1. 读取命令行参数
//! 2. 判断用户想走哪条流程
//! 3. 调用库里的函数并把结果打印出来

use anyhow::Result;
use clap::Parser;
use log::{info, warn};
use pdfxml::{
    export_annotations,
    export_pdf_annotations_to_xfdf,
    load_annotations_from_pdf,
    load_xfdf,
};
use std::path::PathBuf;

/// 命令行参数定义。
///
/// 可以把它理解成：
/// “用户在终端里能传哪些选项，都在这里说明”。
#[derive(Parser, Debug)]
#[command(name = "pdfxml")]
#[command(about = "XFDF 与 PDF 注释互转工具", long_about = None)]
#[command(version = "0.1.0")]
struct Args {
    /// 输入文件路径。
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// 输出文件路径。
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// 目标 PDF 文件路径。
    ///
    /// 只在 “XFDF -> PDF” 模式下生效。
    #[arg(short, long, value_name = "FILE")]
    target_pdf: Option<PathBuf>,

    /// 是否走 “PDF -> XFDF” 模式。
    #[arg(long)]
    from_pdf: bool,

    /// 是否输出更详细的日志。
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // `--verbose` 打开后，日志会更详细，适合排查问题。
    // 不打开时，默认只输出比较简洁的信息。
    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    if !args.input.exists() {
        return Err(anyhow::anyhow!("输入文件不存在: {:?}", args.input));
    }

    // 如果 `from_pdf = true`，表示方向反过来：
    // 不是 “XFDF -> PDF”，而是 “PDF -> XFDF”。
    if args.from_pdf {
        info!("开始从 PDF 导出 XFDF: {:?}", args.input);

        let output_path = match &args.output {
            Some(path) => path.clone(),
            None => {
                let mut default_output = args.input.clone();
                default_output.set_extension("xfdf");
                default_output
            }
        };

        let xfdf_doc = load_annotations_from_pdf(&args.input)?;
        info!("读取完成，发现 {} 条注释", xfdf_doc.annotations.len());

        if xfdf_doc.annotations.is_empty() {
            warn!("警告：未找到任何注释");
        }

        export_pdf_annotations_to_xfdf(&args.input, &output_path)?;

        info!("导出成功，输出文件: {:?}", output_path);
        println!("XFDF 已成功导出到: {}", output_path.display());
        println!("\n导出摘要:");
        println!("  - 注释总数: {}", xfdf_doc.annotations.len());
        return Ok(());
    }

    // 走到这里，就表示当前方向是 “XFDF -> PDF”。
    info!("开始处理 XFDF 文件: {:?}", args.input);

    let output_path = match &args.output {
        Some(path) => path.clone(),
        None => {
            let mut default_output = args.input.clone();
            default_output.set_extension("pdf");
            default_output
        }
    };

    let xfdf_doc = load_xfdf(&args.input)?;
    info!("解析完成，发现 {} 条注释", xfdf_doc.annotations.len());

    if xfdf_doc.annotations.is_empty() {
        warn!("警告：未找到任何注释");
    }

    // 如果给了 `target_pdf`，就把注释合并进已有 PDF。
    // 如果没给，就新建一个 PDF 来放这些注释。
    if let Some(target_path) = &args.target_pdf {
        if !target_path.exists() {
            return Err(anyhow::anyhow!("目标 PDF 文件不存在: {:?}", target_path));
        }
        info!("将注释合并到现有 PDF: {:?}", target_path);
    } else {
        info!("创建新的 PDF 文件: {:?}", output_path);
    }

    export_annotations(&xfdf_doc, args.target_pdf.as_ref(), &output_path)?;

    info!("导出成功，输出文件: {:?}", output_path);
    println!("注释已成功导出到: {}", output_path.display());
    println!("\n导出摘要:");
    println!("  - 注释总数: {}", xfdf_doc.annotations.len());

    let mut type_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for annot in &xfdf_doc.annotations {
        *type_counts.entry(annot.annotation_type()).or_insert(0) += 1;
    }
    for (typ, count) in type_counts.iter() {
        println!("  - {}: {} 条", typ, count);
    }

    Ok(())
}
