//! MG-7 local export — a faithful port of `src/server/local.ts::generateLocalExport`
//! and its section builders (madr/yadr/taskPlan/imagePrompt/confluence).
//!
//! These reproduce the byte shape of the Node exporter so the web `ExportDrawer`
//! sees identical `{ title, content, imagePrompt? }` previews. Graph rendering
//! (digest/mermaid) is reused from scene-core; only the ADR/doc text framing lives
//! here, since it is product-document formatting rather than canvas logic.

use shape_scene_core::{
    graph_text_digest, make_mermaid, selected_subgraph, DecisionGraph, ExportType, GraphNode,
    NodeStatus, NodeType,
};

/// The export scope discriminator from `exportScopeSchema` (group/node/edge/selection).
pub struct ExportScope {
    pub kind: String,
    pub id: Option<String>,
}

/// One generated export, mirroring `ExportOutput`.
pub struct ExportOutput {
    pub title: String,
    pub content: String,
    pub image_prompt: Option<String>,
}

/// Port of `generateLocalExport`. `scope` selects a subgraph (`selection` is
/// treated as the whole group, like the Node code); `export_type` picks the
/// formatter; `group_title` frames the output title.
pub fn generate_local_export(
    graph: &DecisionGraph,
    export_type: ExportType,
    scope: &ExportScope,
    group_title: &str,
) -> ExportOutput {
    // `selection` scope collapses to the whole group (no id), matching local.ts.
    let (scope_kind, scope_id): (&str, Option<&str>) = if scope.kind == "selection" {
        ("group", None)
    } else {
        (scope.kind.as_str(), scope.id.as_deref())
    };
    let scoped = selected_subgraph(graph, scope_kind, scope_id);

    match export_type {
        ExportType::Mermaid => ExportOutput {
            title: format!("{group_title} Mermaid"),
            content: make_mermaid(&scoped),
            image_prompt: None,
        },
        ExportType::Yadr => ExportOutput {
            title: format!("{group_title} YADR"),
            content: yadr_sections(&scoped, group_title),
            image_prompt: None,
        },
        ExportType::ImagePrompt | ExportType::ArchitectureImage => {
            let prompt = image_prompt(&scoped, group_title);
            ExportOutput {
                title: format!("{group_title} Image Prompt"),
                content: prompt.clone(),
                image_prompt: Some(prompt),
            }
        }
        ExportType::ConfluenceHtml => ExportOutput {
            title: format!("{group_title} Confluence Draft"),
            content: confluence_html(&scoped, group_title),
            image_prompt: None,
        },
        ExportType::AiPlanMd => ExportOutput {
            title: format!("{group_title} AI Task Plan"),
            content: task_plan_sections(&scoped, group_title),
            image_prompt: None,
        },
        // madr + design_doc_md both render MADR markdown; only the title differs.
        ExportType::Madr | ExportType::DesignDocMd => ExportOutput {
            title: format!("{group_title} MADR"),
            content: madr_sections(&scoped, group_title),
            image_prompt: None,
        },
    }
}

/// The `contentTypeFor` map from `src/server/index.ts`.
pub fn content_type_for(export_type: ExportType) -> &'static str {
    match export_type {
        ExportType::Yadr => "application/yaml; charset=utf-8",
        ExportType::ImagePrompt | ExportType::ArchitectureImage => "text/markdown; charset=utf-8",
        ExportType::ConfluenceHtml => "text/html; charset=utf-8",
        ExportType::Mermaid => "text/plain; charset=utf-8",
        _ => "text/markdown; charset=utf-8",
    }
}

// ---------------------------------------------------------------------------
// node partition helpers (mirror the `graph.nodes.filter(...)` calls)
// ---------------------------------------------------------------------------

fn nodes_of_type(graph: &DecisionGraph, ty: NodeType) -> Vec<&GraphNode> {
    graph.nodes.iter().filter(|n| n.node_type == ty).collect()
}

fn selected_node(graph: &DecisionGraph) -> Option<&GraphNode> {
    graph.nodes.iter().find(|n| n.status == NodeStatus::Selected)
}

fn chosen_option<'a>(options: &[&'a GraphNode], selected: Option<&'a GraphNode>) -> Option<&'a GraphNode> {
    options
        .iter()
        .find(|n| n.status == NodeStatus::Selected || n.status == NodeStatus::Viable)
        .copied()
        .or_else(|| options.first().copied())
        .or(selected)
}

// ---------------------------------------------------------------------------
// MADR markdown
// ---------------------------------------------------------------------------

fn madr_sections(graph: &DecisionGraph, group_title: &str) -> String {
    let selected = selected_node(graph);
    let decision_title = selected.map(|n| n.title.as_str()).unwrap_or(group_title);
    let options = nodes_of_type(graph, NodeType::Option);
    let chosen = chosen_option(&options, selected);
    let blockers = nodes_of_type(graph, NodeType::Blocker);
    let tradeoffs = nodes_of_type(graph, NodeType::Tradeoff);
    let evidence = nodes_of_type(graph, NodeType::Evidence);
    let mut decisions = nodes_of_type(graph, NodeType::DecisionPoint);
    decisions.extend(nodes_of_type(graph, NodeType::Subdecision));

    let mut drivers: Vec<String> = Vec::new();
    for n in decisions.iter().chain(tradeoffs.iter()).chain(blockers.iter()) {
        drivers.push(format!("{}: {}", n.title, n.summary));
    }
    let positives: Vec<String> = if evidence.is_empty() {
        vec!["The selected option keeps the decision graph explicit and reviewable.".to_string()]
    } else {
        evidence.iter().map(|n| format!("{}: {}", n.title, n.summary)).collect()
    };
    let negatives: Vec<String> = if blockers.is_empty() {
        vec!["Additional review is needed before implementation details are final.".to_string()]
    } else {
        blockers.iter().map(|n| format!("{}: {}", n.title, n.summary)).collect()
    };

    let context = selected
        .map(|n| n.summary.clone())
        .or_else(|| graph.nodes.first().map(|n| n.summary.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Decision graph summary.".to_string());

    let outcome_reason = chosen
        .map(|n| n.summary.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| selected.map(|n| n.summary.clone()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "it best matches the recorded decision drivers.".to_string());
    let chosen_title = chosen.map(|n| n.title.as_str()).unwrap_or(decision_title);

    let mut lines: Vec<String> = vec![
        "---".into(),
        "status: proposed".into(),
        "date:".into(),
        "decision-makers:".into(),
        "consulted:".into(),
        "informed:".into(),
        "---".into(),
        "".into(),
        format!("# {decision_title}"),
        "".into(),
        "## Context and Problem Statement".into(),
        context,
        "".into(),
        "## Decision Drivers".into(),
    ];
    lines.extend(bullet_lines(&drivers));
    lines.push("".into());
    lines.push("## Considered Options".into());
    let option_lines: Vec<String> = options.iter().map(|n| format!("{}: {}", n.title, n.summary)).collect();
    lines.extend(bullet_lines(&option_lines));
    lines.push("".into());
    lines.push("## Decision Outcome".into());
    lines.push(format!(
        "Chosen option: \"{chosen_title}\", because {outcome_reason}"
    ));
    lines.push("".into());
    lines.push("### Consequences".into());
    for line in &positives {
        lines.push(format!("* Good, because {line}"));
    }
    for line in &negatives {
        lines.push(format!("* Bad, because {line}"));
    }
    lines.push("".into());
    lines.push("### Confirmation".into());
    lines.push("Review the accepted group, scene version, and generated artifacts before implementation.".into());
    lines.push("".into());
    lines.push("## Pros and Cons of the Options".into());
    lines.extend(option_pros_and_cons(&options, &evidence, &tradeoffs, &blockers));
    lines.push("".into());
    lines.push("## More Information".into());
    lines.push("```text".into());
    lines.push(graph_text_digest(graph));
    lines.push("```".into());
    lines.join("\n")
}

fn option_pros_and_cons(
    options: &[&GraphNode],
    evidence: &[&GraphNode],
    tradeoffs: &[&GraphNode],
    blockers: &[&GraphNode],
) -> Vec<String> {
    if options.is_empty() {
        return vec![
            "### TODO".into(),
            "".into(),
            "* Good, because TODO".into(),
            "* Neutral, because TODO".into(),
            "* Bad, because TODO".into(),
        ];
    }
    let mut out = Vec::new();
    for option in options {
        out.push(format!("### {}", option.title));
        out.push("".into());
        out.push(option.summary.clone());
        out.push("".into());
        for n in evidence {
            out.push(format!("* Good, because {}: {}", n.title, n.summary));
        }
        if tradeoffs.is_empty() {
            out.push("* Neutral, because additional tradeoffs may emerge during review.".into());
        } else {
            for n in tradeoffs {
                out.push(format!("* Neutral, because {}: {}", n.title, n.summary));
            }
        }
        if blockers.is_empty() {
            out.push("* Bad, because implementation risk has not been fully reviewed.".into());
        } else {
            for n in blockers {
                out.push(format!("* Bad, because {}: {}", n.title, n.summary));
            }
        }
        out.push("".into());
    }
    out
}

// ---------------------------------------------------------------------------
// YADR yaml
// ---------------------------------------------------------------------------

fn yadr_sections(graph: &DecisionGraph, group_title: &str) -> String {
    let selected = selected_node(graph);
    let decision_title = selected.map(|n| n.title.as_str()).unwrap_or(group_title);
    let options = nodes_of_type(graph, NodeType::Option);
    let chosen = chosen_option(&options, selected);
    let blockers = nodes_of_type(graph, NodeType::Blocker);
    let tradeoffs = nodes_of_type(graph, NodeType::Tradeoff);
    let evidence = nodes_of_type(graph, NodeType::Evidence);

    let mut drivers: Vec<(String, String)> = Vec::new();
    for n in nodes_of_type(graph, NodeType::DecisionPoint)
        .iter()
        .chain(nodes_of_type(graph, NodeType::Subdecision).iter())
        .chain(tradeoffs.iter())
        .chain(blockers.iter())
    {
        drivers.push((n.title.clone(), n.summary.clone()));
    }

    let option_entries: Vec<&GraphNode> = if !options.is_empty() {
        options.clone()
    } else if let Some(c) = chosen {
        vec![c]
    } else {
        vec![]
    };

    // positives/negatives fall back to a synthetic (title, summary) pair.
    let positives: Vec<(String, String)> = if evidence.is_empty() {
        vec![("Reviewability".into(), "The decision is captured as a typed graph.".into())]
    } else {
        evidence.iter().map(|n| (n.title.clone(), n.summary.clone())).collect()
    };
    let negatives: Vec<(String, String)> = if blockers.is_empty() {
        vec![("Open review".into(), "Implementation details still need review.".into())]
    } else {
        blockers.iter().map(|n| (n.title.clone(), n.summary.clone())).collect()
    };

    let context = selected
        .map(|n| n.summary.clone())
        .or_else(|| graph.nodes.first().map(|n| n.summary.clone()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Decision graph summary.".to_string());
    let justification = chosen
        .map(|n| n.summary.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| selected.map(|n| n.summary.clone()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "Chosen according to the recorded decision drivers.".to_string());

    let mut lines: Vec<String> = vec![
        "---".into(),
        "metadata:".into(),
        "  status: proposed".into(),
        "  date: TODO".into(),
        "  decision-makers: TODO".into(),
        "  consulted: TODO".into(),
        "  informed: TODO".into(),
        "".into(),
        format!("title: {}", yaml_string(decision_title)),
        "".into(),
        "context-and-problem-statement: |".into(),
    ];
    lines.extend(yaml_block(&context, 2));
    lines.push("".into());
    lines.push("decision-drivers:".into());
    let driver_strs: Vec<String> = drivers.iter().map(|(t, s)| format!("{t}: {s}")).collect();
    lines.extend(yaml_list(&driver_strs, 0));
    lines.push("".into());
    lines.push("considered-options:".into());
    let option_titles: Vec<String> = option_entries.iter().map(|n| n.title.clone()).collect();
    lines.extend(yaml_list(&option_titles, 0));
    lines.push("".into());
    lines.push("pros-and-cons-of-the-options:".into());
    lines.extend(yadr_option_entries(&option_entries, &evidence, &tradeoffs, &blockers));
    lines.push("".into());
    lines.push("decision-outcome:".into());
    lines.push("  chosen-option:".into());
    lines.push(format!(
        "    link: {}",
        yaml_string(chosen.map(|n| n.title.as_str()).unwrap_or(decision_title))
    ));
    lines.push("    justification: |".into());
    lines.extend(yaml_block(&justification, 6));
    lines.push("  consequences:".into());
    lines.push("    positive:".into());
    let pos_strs: Vec<String> = positives.iter().map(|(t, s)| format!("{t}: {s}")).collect();
    lines.extend(yaml_list(&pos_strs, 6));
    lines.push("    neutral: []".into());
    lines.push("    negative:".into());
    let neg_strs: Vec<String> = negatives.iter().map(|(t, s)| format!("{t}: {s}")).collect();
    lines.extend(yaml_list(&neg_strs, 6));
    lines.push("  confirmation: |".into());
    lines.extend(yaml_block(
        "Review the accepted group, scene version, and generated artifacts before implementation.",
        4,
    ));
    lines.push("".into());
    lines.push("more-information: |".into());
    lines.extend(yaml_block(&graph_text_digest(graph), 2));
    lines.join("\n")
}

fn yadr_option_entries(
    options: &[&GraphNode],
    evidence: &[&GraphNode],
    tradeoffs: &[&GraphNode],
    blockers: &[&GraphNode],
) -> Vec<String> {
    if options.is_empty() {
        return vec![
            "  TODO:".into(),
            "    description: TODO".into(),
            "    pros: []".into(),
            "    neutral: []".into(),
            "    cons: []".into(),
        ];
    }
    let mut out = Vec::new();
    for (index, option) in options.iter().enumerate() {
        out.push(format!("  option-{}:", index + 1));
        out.push("    description: |".into());
        out.extend(yaml_block(&option.summary, 6));
        out.push("    pros:".into());
        let pros: Vec<String> = evidence.iter().map(|n| format!("{}: {}", n.title, n.summary)).collect();
        out.extend(yaml_list(&pros, 6));
        out.push("    neutral:".into());
        let neutral: Vec<String> = tradeoffs.iter().map(|n| format!("{}: {}", n.title, n.summary)).collect();
        out.extend(yaml_list(&neutral, 6));
        out.push("    cons:".into());
        let cons: Vec<String> = blockers.iter().map(|n| format!("{}: {}", n.title, n.summary)).collect();
        out.extend(yaml_list(&cons, 6));
    }
    out
}

// ---------------------------------------------------------------------------
// AI task plan
// ---------------------------------------------------------------------------

fn task_plan_sections(graph: &DecisionGraph, title: &str) -> String {
    let tasks = nodes_of_type(graph, NodeType::Task);
    let objective = graph
        .nodes
        .first()
        .map(|n| n.summary.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Implement the accepted group direction.".to_string());
    let mut lines: Vec<String> = vec![
        format!("# {title} AI Task Plan"),
        "".into(),
        "## Objective".into(),
        objective,
        "".into(),
        "## Tasks".into(),
    ];
    if tasks.is_empty() {
        lines.push(
            "1. Use an MCP-connected agent to convert accepted decisions into implementation tasks."
                .into(),
        );
    } else {
        for (index, task) in tasks.iter().enumerate() {
            lines.push(format!("{}. {}: {}", index + 1, task.title, task.summary));
        }
    }
    lines.push("".into());
    lines.push("## Verification".into());
    lines.push("- Validate scene schema.".into());
    lines.push("- Verify exports for the selected group scope.".into());
    lines.push("- Review blockers before implementation.".into());
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// image prompt
// ---------------------------------------------------------------------------

fn image_prompt(graph: &DecisionGraph, title: &str) -> String {
    [
        format!("Create a clean architecture decision diagram for \"{title}\"."),
        "Use labeled boxes for decision points, options, evidence, blockers, tradeoffs, tasks, and artifacts.".to_string(),
        "Use directional arrows for graph relationships and keep labels short enough to read.".to_string(),
        "The result should look like a reviewable architecture diagram, not a web UI screenshot.".to_string(),
        "Prefer a white or very light background, restrained colors, and clear hierarchy.".to_string(),
        "".to_string(),
        "Graph facts:".to_string(),
        graph_text_digest(graph),
    ]
    .join("\n")
}

// ---------------------------------------------------------------------------
// confluence html
// ---------------------------------------------------------------------------

fn confluence_html(graph: &DecisionGraph, group_title: &str) -> String {
    let body: Vec<String> = madr_sections(graph, group_title)
        .split('\n')
        .map(|line| {
            if line.starts_with('#') {
                let heading = line.trim_start_matches('#').trim_start();
                format!("<h2>{}</h2>", escape_html(heading))
            } else {
                format!("<p>{}</p>", escape_html(line))
            }
        })
        .collect();
    format!("<h1>{}</h1>{}", escape_html(group_title), body.join("\n"))
}

// ---------------------------------------------------------------------------
// shared formatting helpers
// ---------------------------------------------------------------------------

fn bullet_lines(lines: &[String]) -> Vec<String> {
    if lines.is_empty() {
        vec!["* TODO".to_string()]
    } else {
        lines.iter().map(|l| format!("* {l}")).collect()
    }
}

fn yaml_list(values: &[String], indent: usize) -> Vec<String> {
    let prefix = " ".repeat(indent);
    if values.is_empty() {
        vec![format!("{prefix}- TODO")]
    } else {
        values.iter().map(|v| format!("{prefix}- {}", yaml_string(v))).collect()
    }
}

fn yaml_block(value: &str, indent: usize) -> Vec<String> {
    let prefix = " ".repeat(indent);
    value.split('\n').map(|line| format!("{prefix}{line}")).collect()
}

/// Port of `yamlString`: bare value if it matches `^[a-zA-Z0-9 _.-]+$`, else
/// JSON-quoted (matching `JSON.stringify` for a string).
fn yaml_string(value: &str) -> String {
    let bare = !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '.' || c == '-'
        });
    if bare {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
    }
}

/// Port of `escapeHtml` from `local.ts`.
fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#039;"),
            other => out.push(other),
        }
    }
    out
}
