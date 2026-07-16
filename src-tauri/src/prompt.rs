//! System prompt assembly (§3). Built at runtime from whatever tools are
//! mounted — never a hardcoded tool list. At M2 the mounted set is empty;
//! MCP servers (M5) feed it.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDesc {
    pub name: String,
    pub description: String,
}

pub fn build_system_prompt(tools: &[ToolDesc]) -> String {
    let mut prompt = String::from(
        "You are Ocellum, a desktop companion for a salesperson. You help with \
         small, high-frequency sales tasks: researching leads, drafting outreach \
         emails, and reminders. Be concise and practical. Never invent facts \
         about a lead; say when you don't know. Drafts are delivered to the \
         clipboard — you never send email.",
    );
    if !tools.is_empty() {
        prompt.push_str("\n\nYou have access to the following tools:\n");
        for t in tools {
            prompt.push_str(&format!("- {}: {}\n", t.name, t.description));
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_reflects_mounted_tool_set() {
        let empty = build_system_prompt(&[]);
        assert!(!empty.contains("following tools"));
        let with = build_system_prompt(&[ToolDesc {
            name: "crm_lookup".into(),
            description: "Look up a lead in the CRM".into(),
        }]);
        assert!(with.contains("crm_lookup"));
        assert_ne!(empty, with, "prompt must change when tools mount");
    }
}
