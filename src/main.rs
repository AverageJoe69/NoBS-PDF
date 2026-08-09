use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "pdfdoctor", version, about = "NoBS PDF inspection engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, ValueEnum)]
enum DocumentTargetArg {
    Original,
    #[value(name = "4k")]
    Screen4k,
    #[value(name = "1440p")]
    Screen1440p,
    #[value(name = "1080p")]
    Screen1080p,
    #[value(name = "720p")]
    Screen720p,
    Custom,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a PDF and write a versioned JSON report to stdout.
    Inspect { input: PathBuf },
    /// Produce a read-only, explainable optimisation plan.
    Plan {
        input: PathBuf,
        /// Resolution target used for conservative classification.
        #[arg(long, default_value_t = 300)]
        target_dpi: u16,
        /// Screen target based on per-page rendered pixel occupancy (separate from print DPI).
        #[arg(long, value_enum)]
        document_target: Option<DocumentTargetArg>,
        /// Required only with --document-target custom.
        #[arg(long)]
        custom_long_dimension: Option<u32>,
    },
    /// Export a validated PDF with conservative 1080p raster downsampling.
    Export {
        input: PathBuf,
        #[arg(long, value_enum)]
        document_target: DocumentTargetArg,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Merge provably safe raster-first page content into one raster layer.
    RasterMerge {
        input: PathBuf,
        #[arg(long, value_enum)]
        document_target: DocumentTargetArg,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// Export each page as a screen-resolution raster while preserving page boxes and annotations.
    FlattenPages {
        input: PathBuf,
        #[arg(long, value_enum)]
        document_target: DocumentTargetArg,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        dry_run: bool,
        /// Override the bundled/local PDFium dynamic library.
        #[arg(long)]
        pdfium_library: Option<PathBuf>,
    },
    /// Run the frozen optimisation regression benchmark and emit measured JSON.
    Benchmark {
        input: PathBuf,
        #[arg(long, value_enum)]
        document_target: DocumentTargetArg,
        /// Override the bundled/local PDFium dynamic library.
        #[arg(long)]
        pdfium_library: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Command::Plan {
        document_target: Some(DocumentTargetArg::Custom),
        custom_long_dimension,
        ..
    } = &cli.command
    {
        if custom_long_dimension.is_none_or(|value| value == 0) {
            eprintln!("pdfdoctor: --document-target custom requires --custom-long-dimension greater than zero");
            std::process::exit(2);
        }
    }
    let result: anyhow::Result<()> = match cli.command {
        Command::Inspect { input } => pdfdoctor::inspect(input)
            .map(|report| {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).expect("serializable report")
                );
            })
            .map_err(Into::into),
        Command::Plan {
            input,
            target_dpi,
            document_target,
            custom_long_dimension,
        } => pdfdoctor::inspect(input)
            .map(|inspection| {
                let document_target = document_target.map(|target| match target {
                    DocumentTargetArg::Original => {
                        pdfdoctor::planner::DocumentTargetProfile::Original
                    }
                    DocumentTargetArg::Screen4k => {
                        pdfdoctor::planner::DocumentTargetProfile::Screen4k
                    }
                    DocumentTargetArg::Screen1440p => {
                        pdfdoctor::planner::DocumentTargetProfile::Screen1440p
                    }
                    DocumentTargetArg::Screen1080p => {
                        pdfdoctor::planner::DocumentTargetProfile::Screen1080p
                    }
                    DocumentTargetArg::Screen720p => {
                        pdfdoctor::planner::DocumentTargetProfile::Screen720p
                    }
                    DocumentTargetArg::Custom => {
                        pdfdoctor::planner::DocumentTargetProfile::Custom {
                            long_dimension_px: custom_long_dimension.unwrap_or(0),
                        }
                    }
                });
                let plan = pdfdoctor::create_plan(
                    &inspection,
                    &pdfdoctor::PlannerConfig {
                        target_dpi,
                        document_target,
                        ..Default::default()
                    },
                );
                eprintln!("{}", pdfdoctor::planner::human_summary(&plan));
                println!(
                    "{}",
                    serde_json::to_string_pretty(&plan).expect("serializable plan")
                );
            })
            .map_err(Into::into),
        Command::Export {
            input,
            document_target,
            output,
            dry_run,
        } => {
            if !matches!(document_target, DocumentTargetArg::Screen1080p) {
                Err(anyhow::anyhow!(
                    "this conservative exporter currently supports only --document-target 1080p"
                ))
            } else {
                pdfdoctor::exporter::export_1080p(
                    &input,
                    &output,
                    &pdfdoctor::exporter::ExportOptions { dry_run },
                )
                .map(|report| {
                    eprintln!("{}", pdfdoctor::exporter::human_export_summary(&report));
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).expect("serializable export report")
                    );
                })
                .map_err(Into::into)
            }
        }
        Command::RasterMerge {
            input,
            document_target,
            output,
            dry_run,
        } => {
            if !matches!(document_target, DocumentTargetArg::Screen1080p) {
                Err(anyhow::anyhow!(
                    "raster merge currently supports only --document-target 1080p"
                ))
            } else {
                pdfdoctor::raster_merge::merge_1080p(&input, &output, dry_run)
                    .map(|report| {
                        eprintln!("{}", pdfdoctor::raster_merge::human_summary(&report));
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&report)
                                .expect("serializable raster merge report")
                        );
                    })
                    .map_err(Into::into)
            }
        }
        Command::FlattenPages {
            input,
            document_target,
            output,
            dry_run,
            pdfium_library,
        } => {
            if !matches!(document_target, DocumentTargetArg::Screen1080p) {
                Err(anyhow::anyhow!(
                    "full-page raster export currently supports only --document-target 1080p"
                ))
            } else {
                pdfdoctor::flatten_pages::flatten_1080p(
                    &input,
                    &output,
                    dry_run,
                    pdfium_library.as_deref(),
                )
                .map(|report| {
                    eprintln!("{}", pdfdoctor::flatten_pages::human_summary(&report));
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).expect("serializable flatten report")
                    );
                })
                .map_err(Into::into)
            }
        }
        Command::Benchmark {
            input,
            document_target,
            pdfium_library,
        } => {
            if !matches!(document_target, DocumentTargetArg::Screen1080p) {
                Err(anyhow::anyhow!(
                    "the frozen benchmark currently supports only --document-target 1080p"
                ))
            } else {
                pdfdoctor::benchmark::run_1080p(&input, pdfium_library.as_deref())
                    .and_then(|report| {
                        eprintln!("{}", pdfdoctor::benchmark::human_summary(&report));
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&report)
                                .expect("serializable benchmark report")
                        );
                        if report.validation.passed {
                            Ok(())
                        } else {
                            Err(pdfdoctor::benchmark::BenchmarkError::ValidationFailed)
                        }
                    })
                    .map_err(Into::into)
            }
        }
    };
    match result {
        Ok(()) => {}
        Err(error) => {
            eprintln!("pdfdoctor: {error}");
            std::process::exit(1);
        }
    }
}
