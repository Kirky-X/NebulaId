use nebula_code_review::CodeReviewSystem;
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "nebula-code-review")]
/// Multi-agent code review tool for Nebula ID project
struct Args {
    /// Path to the project root (defaults to current directory)
    #[structopt(short, long, parse(from_os_str))]
    project: Option<PathBuf>,

    /// Output format (json, text, markdown)
    #[structopt(short, long, default_value = "text")]
    format: String,

    /// Run specific agent (security, performance, quality, architecture)
    #[structopt(short, long)]
    _agent: Option<String>,

    /// Show only issues with severity >= threshold (critical, high, medium, low)
    #[structopt(short, long, default_value = "low")]
    _threshold: String,

    /// Generate JSON output file
    #[structopt(short, long, parse(from_os_str))]
    output: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::from_args();

    let project_root = args
        .project
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    println!("Nebula ID Code Review Tool");
    println!("==========================");
    println!("Project: {:?}", project_root);
    println!();

    let review_system = CodeReviewSystem::new(&project_root);

    // Run all reviews
    let results = review_system.run_full_review().await;

    // Process results based on output format
    match args.format.as_str() {
        "json" => {
            let json_output = serde_json::to_string_pretty(&results).unwrap();
            println!("{}", json_output);

            if let Some(output_path) = &args.output {
                std::fs::write(output_path, json_output).expect("Failed to write output file");
                println!("\nJSON output written to: {:?}", output_path);
            }
        }
        "markdown" => {
            generate_markdown_report(&results);
        }
        _ => {
            // Default text format
            print_text_report(&results);
        }
    }
}

fn print_text_report(results: &[nebula_code_review::CodeReviewResult]) {
    let mut total_score = 0.0;
    let mut total_issues = 0;
    let mut critical = 0;
    let mut high = 0;
    let mut medium = 0;
    let mut low = 0;

    for result in results {
        total_score += result.summary.score;
        total_issues += result.summary.total_issues;
        critical += result.summary.critical_count;
        high += result.summary.high_count;
        medium += result.summary.medium_count;
        low += result.summary.low_count;

        println!("\n📊 Agent: {}", result.agent_name);
        println!("   Score: {:.1}/100", result.summary.score);
        println!(
            "   Issues: {} ({} critical, {} high, {} medium, {} low)",
            result.summary.total_issues,
            result.summary.critical_count,
            result.summary.high_count,
            result.summary.medium_count,
            result.summary.low_count
        );

        if !result.findings.is_empty() {
            println!("\n   🔍 Findings:");
            for finding in &result.findings {
                let severity_icon = match finding.severity {
                    nebula_code_review::Severity::Critical => "🔴",
                    nebula_code_review::Severity::High => "🟠",
                    nebula_code_review::Severity::Medium => "🟡",
                    nebula_code_review::Severity::Low => "🟢",
                    nebula_code_review::Severity::Info => "🔵",
                };

                println!(
                    "   {} [{}] {}",
                    severity_icon,
                    finding.category.to_uppercase(),
                    finding.message
                );
                if let Some(line) = finding.line {
                    println!("      📍 {}:{}", finding.file, line);
                } else {
                    println!("      📍 {}", finding.file);
                }
                println!("      💡 {}", finding.suggestion);

                if let Some(snippet) = &finding.code_snippet {
                    println!("      ```rust");
                    println!(
                        "      {}",
                        snippet.lines().take(3).collect::<Vec<_>>().join("\n      ")
                    );
                    println!("      ```");
                }
            }
        }
    }

    // Summary
    println!("\n");
    println!("═══════════════════════════════════════════════════");
    println!("                    SUMMARY                         ");
    println!("═══════════════════════════════════════════════════");
    println!("Total Agents: {}", results.len());
    println!(
        "Average Score: {:.1}/100",
        total_score / results.len() as f64
    );
    println!("Total Issues: {}", total_issues);
    println!("  🔴 Critical: {}", critical);
    println!("  🟠 High: {}", high);
    println!("  🟡 Medium: {}", medium);
    println!("  🟢 Low: {}", low);
    println!("═══════════════════════════════════════════════════");

    // Recommendations
    if critical > 0 {
        println!("\n⚠️  CRITICAL ISSUES FOUND - Immediate attention required!");
    }
    if high > 5 {
        println!("\n⚠️  High number of high-severity issues - Review recommended");
    }
}

fn generate_markdown_report(results: &[nebula_code_review::CodeReviewResult]) {
    println!("# Nebula ID Code Review Report\n");
    println!(
        "Generated: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    let total_score: f64 = results.iter().map(|r| r.summary.score).sum();
    let avg_score = total_score / results.len() as f64;

    println!("## Summary\n");
    println!("| Metric | Value |");
    println!("|--------|-------|");
    println!("| Average Score | {:.1}/100 |", avg_score);
    println!(
        "| Total Issues | {} |",
        results
            .iter()
            .map(|r| r.summary.total_issues)
            .sum::<usize>()
    );
    println!(
        "| Critical Issues | {} |",
        results
            .iter()
            .map(|r| r.summary.critical_count)
            .sum::<usize>()
    );
    println!(
        "| High Issues | {} |",
        results.iter().map(|r| r.summary.high_count).sum::<usize>()
    );
    println!("\n");

    for result in results {
        println!("## {} Agent\n", result.agent_name.to_uppercase());
        println!("**Score:** {:.1}/100\n", result.summary.score);

        if !result.findings.is_empty() {
            println!("| Severity | Category | Message | Location |");
            println!("|----------|----------|---------|----------|");

            for finding in &result.findings {
                let severity = match finding.severity {
                    nebula_code_review::Severity::Critical => "🔴 Critical",
                    nebula_code_review::Severity::High => "🟠 High",
                    nebula_code_review::Severity::Medium => "🟡 Medium",
                    nebula_code_review::Severity::Low => "🟢 Low",
                    nebula_code_review::Severity::Info => "🔵 Info",
                };

                let location = match finding.line {
                    Some(line) => format!("{}:{}", finding.file, line),
                    None => finding.file.clone(),
                };

                println!(
                    "| {} | {} | {} | {} |",
                    severity, finding.category, finding.message, location
                );
            }
            println!();
        } else {
            println!("No issues found. ✅\n");
        }
    }
}
