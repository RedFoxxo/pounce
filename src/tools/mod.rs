//! Tools, grouped by tier and toolset. The name prefix is the permission contract
//! (`read_` / `write_` / `delete_` / `admin_`); tool names, parameter names and env
//! vars are a contract with the Mamami plugin (approval rules, skills).

pub mod context;
pub mod generic;
pub mod params;
pub mod respond;
pub mod workflow;

pub use context::{Clock, Ctx};
pub use respond::{Answer, ToolResult, failed, failure, invalid, success};

use serde_json::{Value, json};

use crate::http::BoxFuture;
use crate::schema::{self, Args, Field};

pub type Handler = for<'a> fn(&'a Ctx, Args) -> BoxFuture<'a, Answer>;

pub struct Tool {
    /// Registered without a `pounce_` prefix; the tier prefix is the permission contract.
    pub name: &'static str,
    pub description: &'static str,
    pub fields: Vec<Field>,
    pub handler: Handler,
}

/// A handler from a closure-like block: `handler!(|ctx, args| { ... })`.
macro_rules! handler {
    (|$ctx:ident, $args:ident| $body:block) => {{
        #[allow(unused_variables)]
        fn handle<'a>(
            $ctx: &'a $crate::tools::Ctx,
            $args: $crate::schema::Args,
        ) -> $crate::http::BoxFuture<'a, $crate::tools::Answer> {
            Box::pin(async move $body)
        }
        handle
    }};
}
pub(crate) use handler;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Read,
    Write,
    Delete,
    Admin,
}

pub fn tier_of(name: &str) -> Option<Tier> {
    match name.split('_').next() {
        Some("read") => Some(Tier::Read),
        Some("write") => Some(Tier::Write),
        Some("delete") => Some(Tier::Delete),
        Some("admin") => Some(Tier::Admin),
        _ => None,
    }
}

/// Groups of tools a server exposes (`POUNCE_TOOLSETS`): `cards` are the layer 2
/// workflow tools, `raw` the layer 1 catalog tools, `admin` the `admin_` tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolset {
    Cards,
    Raw,
    Admin,
}

pub const TOOLSETS: [Toolset; 3] = [Toolset::Cards, Toolset::Raw, Toolset::Admin];

impl Toolset {
    pub fn name(self) -> &'static str {
        match self {
            Toolset::Cards => "cards",
            Toolset::Raw => "raw",
            Toolset::Admin => "admin",
        }
    }
}

fn group(set: Toolset) -> Vec<Tool> {
    match set {
        Toolset::Cards => {
            let mut tools = workflow::read::tools();
            tools.extend(workflow::write::tools());
            tools.extend(workflow::delete::tools());
            tools.extend(generic::extras::card_tools());
            tools
        }
        Toolset::Raw => {
            let mut tools = generic::read::read_tools();
            tools.extend(generic::write::write_tools());
            tools.extend(generic::write::delete_tools());
            tools.extend(generic::extras::raw_tools());
            tools
        }
        Toolset::Admin => {
            let mut tools = generic::write::admin_tools();
            tools.extend(generic::extras::admin_tools());
            tools
        }
    }
}

/// The tools of the given toolsets, in registration order.
pub fn tools_for(sets: &[Toolset]) -> Vec<Tool> {
    let tools: Vec<Tool> = TOOLSETS
        .into_iter()
        .filter(|t| sets.contains(t))
        .flat_map(group)
        .collect();
    for (i, tool) in tools.iter().enumerate() {
        assert!(
            tier_of(tool.name).is_some(),
            "Tool \"{}\" has no tier prefix (read_/write_/delete_/admin_)",
            tool.name
        );
        assert!(
            tools[..i].iter().all(|t| t.name != tool.name),
            "Tool \"{}\" is registered twice",
            tool.name
        );
    }
    tools
}

/// The `tools/list` answer. Only reads are marked; everything else keeps the MCP
/// defaults (not read-only, destructive, open world).
pub fn list(tools: &[Tool]) -> Value {
    let listed: Vec<Value> = tools
        .iter()
        .map(|t| {
            let mut tool = json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": schema::input_schema(&t.fields),
            });
            if tier_of(t.name) == Some(Tier::Read) {
                tool["annotations"] = json!({ "readOnlyHint": true });
            }
            tool
        })
        .collect();
    json!({ "tools": listed })
}

/// Validates the arguments and runs the tool. Never fails: every outcome is an answer.
pub async fn call(ctx: &Ctx, tools: &[Tool], name: &str, args: &Value) -> ToolResult {
    let Some(tool) = tools.iter().find(|t| t.name == name) else {
        return ToolResult {
            text: format!("MCP error -32602: Tool {name} not found"),
            is_error: true,
        };
    };
    match schema::validate(&tool.fields, args) {
        Err(problems) => ToolResult {
            text: format!(
                "MCP error -32602: Input validation error: Invalid arguments for tool {name}: {problems}"
            ),
            is_error: true,
        },
        Ok(args) => (tool.handler)(ctx, args)
            .await
            .unwrap_or_else(|early| early),
    }
}
