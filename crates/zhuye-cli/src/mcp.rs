pub fn qualified_mcp_tool_name_prefix(_server: &str, _tool: &str) -> String {
    format!("mcp:{}:{}", _server, _tool)
}
