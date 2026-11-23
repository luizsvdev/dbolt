use std::collections::{BTreeMap, HashSet};

use dbolt_database::{ConnectionId, ConnectionProfile, ConnectionScope, TableInfo, TableKind};
use gpui_kit::SharedString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load<T> {
    NotLoaded,
    Loading,
    Loaded(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn needs_load(&self) -> bool {
        matches!(self, Self::NotLoaded | Self::Failed(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaEntry {
    pub name: String,
    pub tables: Load<Vec<TableInfo>>,
}

impl SchemaEntry {
    pub fn new(name: String) -> Self {
        Self {
            name,
            tables: Load::NotLoaded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionContent {
    Schemas(Vec<SchemaEntry>),
    Tables(Vec<TableInfo>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionEntry {
    pub profile: ConnectionProfile,
    pub status: ConnectionStatus,
    pub content: Load<ConnectionContent>,
}

impl ConnectionEntry {
    pub fn new(profile: ConnectionProfile) -> Self {
        Self {
            profile,
            status: ConnectionStatus::Disconnected,
            content: Load::NotLoaded,
        }
    }

    pub fn schema_mut(&mut self, name: &str) -> Option<&mut SchemaEntry> {
        match &mut self.content {
            Load::Loaded(ConnectionContent::Schemas(schemas)) => schemas
                .iter_mut()
                .find(|schema| schema.name == name),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Group {
        path: String,
    },
    Connection {
        id: ConnectionId,
        scope: ConnectionScope,
        status: ConnectionStatus,
    },
    Schema {
        connection: ConnectionId,
        name: String,
    },
    Table {
        kind: TableKind,
    },
    Loading,
    Error(String),
    Empty,
    PendingFolder,
}

impl NodeKind {
    pub fn is_status(&self) -> bool {
        matches!(self, Self::Loading | Self::Error(_) | Self::Empty)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub id: SharedString,
    pub label: SharedString,
    pub kind: NodeKind,
    pub expanded: bool,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    fn leaf(id: SharedString, label: impl Into<SharedString>, kind: NodeKind) -> Self {
        Self {
            id,
            label: label.into(),
            kind,
            expanded: false,
            children: Vec::new(),
        }
    }
}

pub fn group_node_id(path: &str) -> SharedString {
    format!("group:{path}").into()
}

pub const PENDING_FOLDER_ID: &str = "pending-folder";

/// Falls back to the top level when the parent is hidden by a search.
pub fn insert_pending_folder(nodes: &mut Vec<TreeNode>, parent: Option<&str>) {
    let pending = TreeNode::leaf(
        PENDING_FOLDER_ID.into(),
        SharedString::default(),
        NodeKind::PendingFolder,
    );
    let parent_node = parent.and_then(|parent| find_node_mut(nodes, &group_node_id(parent)));
    match parent_node {
        Some(parent_node) => {
            parent_node.expanded = true;
            parent_node
                .children
                .insert(0, pending);
        }
        None => nodes.insert(0, pending),
    }
}

fn find_node_mut<'a>(nodes: &'a mut [TreeNode], id: &SharedString) -> Option<&'a mut TreeNode> {
    for node in nodes {
        if &node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node_mut(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

pub fn folder_path(
    parent: Option<&str>,
    name: &str,
    existing_folders: &[String],
) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a folder name");
    }
    if name.contains('/') {
        return Err("A folder name cannot contain \"/\"");
    }
    let path = match parent {
        Some(parent) => format!("{parent}/{name}"),
        None => name.to_string(),
    };
    if existing_folders.contains(&path) {
        return Err("A folder with this name already exists here");
    }
    Ok(path)
}

pub fn connection_node_id(connection: ConnectionId) -> SharedString {
    format!("connection:{connection}").into()
}

pub fn schema_node_id(connection: ConnectionId, schema: &str) -> SharedString {
    format!("schema:{connection}:{schema}").into()
}

fn table_node_id(connection: ConnectionId, schema: Option<&str>, table: &str) -> SharedString {
    // The schema length keeps ids unique when names themselves contain `:`.
    let schema = schema.unwrap_or_default();
    format!("table:{connection}:{}:{schema}:{table}", schema.len()).into()
}

fn status_node_id(parent: &SharedString) -> SharedString {
    format!("{parent}#status").into()
}

pub fn group_segments(group: Option<&str>) -> Vec<&str> {
    group
        .map(|group| {
            group
                .split('/')
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn build_tree(
    connections: &[ConnectionEntry],
    folders: &[String],
    expanded: &HashSet<SharedString>,
    filter: &str,
) -> Vec<TreeNode> {
    let mut sorted: Vec<&ConnectionEntry> = connections.iter().collect();
    sorted.sort_by_cached_key(|entry| {
        entry
            .profile
            .name
            .to_lowercase()
    });

    let mut root = GroupBuilder::default();
    for folder in folders {
        root.ensure(&group_segments(Some(folder)));
    }
    for entry in sorted {
        let segments = group_segments(entry.profile.group.as_deref());
        root.insert(&segments, connection_node(entry, expanded));
    }
    let nodes = root.into_nodes("", expanded);

    let query = filter.trim().to_lowercase();
    if query.is_empty() {
        return nodes;
    }
    nodes
        .into_iter()
        .filter_map(|node| filter_node(node, &query))
        .collect()
}

#[derive(Default)]
struct GroupBuilder {
    groups: BTreeMap<String, GroupBuilder>,
    connections: Vec<TreeNode>,
}

impl GroupBuilder {
    fn ensure(&mut self, segments: &[&str]) {
        if let Some((first, rest)) = segments.split_first() {
            self.groups
                .entry((*first).to_string())
                .or_default()
                .ensure(rest);
        }
    }

    fn insert(&mut self, segments: &[&str], node: TreeNode) {
        match segments.split_first() {
            Some((first, rest)) => self
                .groups
                .entry((*first).to_string())
                .or_default()
                .insert(rest, node),
            None => self.connections.push(node),
        }
    }

    fn into_nodes(self, prefix: &str, expanded: &HashSet<SharedString>) -> Vec<TreeNode> {
        let mut nodes = Vec::with_capacity(self.groups.len() + self.connections.len());
        for (name, group) in self.groups {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let id = group_node_id(&path);
            nodes.push(TreeNode {
                expanded: expanded.contains(&id),
                children: group.into_nodes(&path, expanded),
                id,
                label: name.into(),
                kind: NodeKind::Group { path },
            });
        }
        nodes.extend(self.connections);
        nodes
    }
}

fn connection_node(entry: &ConnectionEntry, expanded: &HashSet<SharedString>) -> TreeNode {
    let connection = entry.profile.id;
    let id = connection_node_id(connection);
    let status_id = status_node_id(&id);

    let children = match (&entry.status, &entry.content) {
        (ConnectionStatus::Connecting, _) => {
            vec![TreeNode::leaf(status_id, "Connecting…", NodeKind::Loading)]
        }
        (ConnectionStatus::Failed(message), _) => vec![TreeNode::leaf(
            status_id,
            message.clone(),
            NodeKind::Error(message.clone()),
        )],
        (_, Load::NotLoaded | Load::Loading) => {
            vec![TreeNode::leaf(status_id, "Loading…", NodeKind::Loading)]
        }
        (_, Load::Failed(message)) => vec![TreeNode::leaf(
            status_id,
            message.clone(),
            NodeKind::Error(message.clone()),
        )],
        (_, Load::Loaded(ConnectionContent::Schemas(schemas))) if schemas.is_empty() => {
            vec![TreeNode::leaf(status_id, "No schemas", NodeKind::Empty)]
        }
        (_, Load::Loaded(ConnectionContent::Schemas(schemas))) => schemas
            .iter()
            .map(|schema| schema_node(connection, schema, expanded))
            .collect(),
        (_, Load::Loaded(ConnectionContent::Tables(tables))) => {
            table_nodes(connection, None, &status_id, tables)
        }
    };

    TreeNode {
        expanded: expanded.contains(&id),
        id,
        label: entry
            .profile
            .name
            .clone()
            .into(),
        kind: NodeKind::Connection {
            id: connection,
            scope: entry.profile.scope,
            status: entry.status.clone(),
        },
        children,
    }
}

fn schema_node(
    connection: ConnectionId,
    schema: &SchemaEntry,
    expanded: &HashSet<SharedString>,
) -> TreeNode {
    let id = schema_node_id(connection, &schema.name);
    let status_id = status_node_id(&id);
    let children = match &schema.tables {
        Load::NotLoaded | Load::Loading => {
            vec![TreeNode::leaf(status_id, "Loading…", NodeKind::Loading)]
        }
        Load::Failed(message) => vec![TreeNode::leaf(
            status_id,
            message.clone(),
            NodeKind::Error(message.clone()),
        )],
        Load::Loaded(tables) => table_nodes(connection, Some(&schema.name), &status_id, tables),
    };

    TreeNode {
        expanded: expanded.contains(&id),
        id,
        label: schema.name.clone().into(),
        kind: NodeKind::Schema {
            connection,
            name: schema.name.clone(),
        },
        children,
    }
}

fn table_nodes(
    connection: ConnectionId,
    schema: Option<&str>,
    status_id: &SharedString,
    tables: &[TableInfo],
) -> Vec<TreeNode> {
    if tables.is_empty() {
        return vec![TreeNode::leaf(
            status_id.clone(),
            "No tables",
            NodeKind::Empty,
        )];
    }
    tables
        .iter()
        .map(|table| {
            TreeNode::leaf(
                table_node_id(connection, schema, &table.name),
                table.name.clone(),
                NodeKind::Table { kind: table.kind },
            )
        })
        .collect()
}

fn filter_node(mut node: TreeNode, query: &str) -> Option<TreeNode> {
    if !node.kind.is_status()
        && node
            .label
            .to_lowercase()
            .contains(query)
    {
        return Some(node);
    }
    let children: Vec<TreeNode> = std::mem::take(&mut node.children)
        .into_iter()
        .filter_map(|child| filter_node(child, query))
        .collect();
    if children.is_empty() {
        return None;
    }
    node.children = children;
    node.expanded = true;
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, group: Option<&str>) -> ConnectionProfile {
        let mut profile = ConnectionProfile::new(name, "test-driver", ConnectionScope::Project);
        profile.group = group.map(str::to_string);
        profile
    }

    fn table(name: &str, kind: TableKind) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            kind,
        }
    }

    fn labels(nodes: &[TreeNode]) -> Vec<&str> {
        nodes
            .iter()
            .map(|node| node.label.as_ref())
            .collect()
    }

    fn loaded_entry(name: &str) -> ConnectionEntry {
        let mut entry = ConnectionEntry::new(profile(name, None));
        entry.status = ConnectionStatus::Connected;
        let mut public = SchemaEntry::new("public".to_string());
        public.tables = Load::Loaded(vec![
            table("users", TableKind::Table),
            table("active_users", TableKind::View),
        ]);
        let mut audit = SchemaEntry::new("audit".to_string());
        audit.tables = Load::Loaded(vec![table("events", TableKind::Table)]);
        entry.content = Load::Loaded(ConnectionContent::Schemas(vec![audit, public]));
        entry
    }

    #[test]
    fn group_segments_ignore_empty_parts() {
        assert_eq!(group_segments(Some(" a//b/ ")), vec!["a", "b"]);
        assert!(group_segments(Some("/")).is_empty());
        assert!(group_segments(None).is_empty());
    }

    #[test]
    fn pending_folder_goes_first_inside_its_parent() {
        let mut nodes = build_tree(&[], &["work".to_string()], &HashSet::new(), "");

        insert_pending_folder(&mut nodes, Some("work"));

        assert!(nodes[0].expanded);
        assert_eq!(
            nodes[0].children[0]
                .id
                .as_ref(),
            PENDING_FOLDER_ID
        );
    }

    #[test]
    fn folder_path_nests_under_parent_and_trims() {
        assert_eq!(
            folder_path(Some("work"), " prod ", &[]),
            Ok("work/prod".to_string())
        );
        assert_eq!(folder_path(None, "work", &[]), Ok("work".to_string()));
    }

    #[test]
    fn folder_path_rejects_empty_slashes_and_duplicates() {
        assert!(folder_path(None, "  ", &[]).is_err());
        assert!(folder_path(None, "a/b", &[]).is_err());
        assert!(folder_path(Some("work"), "prod", &["work/prod".to_string()]).is_err());
    }

    #[test]
    fn empty_folders_are_kept() {
        let nodes = build_tree(&[], &["work/prod".to_string()], &HashSet::new(), "");

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, group_node_id("work"));
        assert_eq!(nodes[0].children[0].id, group_node_id("work/prod"));
    }

    #[test]
    fn groups_come_first_and_nest() {
        let connections = vec![
            ConnectionEntry::new(profile("zeta", None)),
            ConnectionEntry::new(profile("beta", Some("work/prod"))),
            ConnectionEntry::new(profile("Alpha", None)),
            ConnectionEntry::new(profile("gamma", Some("work"))),
        ];

        let nodes = build_tree(&connections, &[], &HashSet::new(), "");

        assert_eq!(labels(&nodes), vec!["work", "Alpha", "zeta"]);
        let work = &nodes[0];
        assert_eq!(work.id, group_node_id("work"));
        assert_eq!(labels(&work.children), vec!["prod", "gamma"]);
        assert_eq!(work.children[0].id, group_node_id("work/prod"));
        assert_eq!(labels(&work.children[0].children), vec!["beta"]);
    }

    #[test]
    fn unloaded_connection_is_expandable() {
        let connections = vec![ConnectionEntry::new(profile("local", None))];

        let nodes = build_tree(&connections, &[], &HashSet::new(), "");

        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].children[0].kind, NodeKind::Loading);
    }

    #[test]
    fn failed_connection_shows_error_child() {
        let mut entry = ConnectionEntry::new(profile("local", None));
        entry.status = ConnectionStatus::Failed("password authentication failed".to_string());

        let nodes = build_tree(&[entry], &[], &HashSet::new(), "");

        assert_eq!(
            nodes[0].children[0].kind,
            NodeKind::Error("password authentication failed".to_string())
        );
    }

    #[test]
    fn expanded_set_controls_expansion() {
        let entry = loaded_entry("local");
        let connection = entry.profile.id;
        let expanded = HashSet::from([
            connection_node_id(connection),
            schema_node_id(connection, "public"),
        ]);

        let nodes = build_tree(&[entry], &[], &expanded, "");

        let connection_node = &nodes[0];
        assert!(connection_node.expanded);
        assert_eq!(labels(&connection_node.children), vec!["audit", "public"]);
        assert!(!connection_node.children[0].expanded);
        assert!(connection_node.children[1].expanded);
        assert_eq!(
            labels(&connection_node.children[1].children),
            vec!["users", "active_users"]
        );
    }

    #[test]
    fn empty_lists_show_a_status_row() {
        let mut entry = ConnectionEntry::new(profile("local", None));
        entry.status = ConnectionStatus::Connected;
        entry.content = Load::Loaded(ConnectionContent::Tables(Vec::new()));

        let nodes = build_tree(&[entry], &[], &HashSet::new(), "");

        assert_eq!(nodes[0].children[0].kind, NodeKind::Empty);
    }

    #[test]
    fn filter_keeps_matches_and_expanded_ancestors() {
        let entry = loaded_entry("local");
        let other = ConnectionEntry::new(profile("other", Some("team")));

        let nodes = build_tree(&[entry, other], &[], &HashSet::new(), "USER");

        assert_eq!(labels(&nodes), vec!["local"]);
        let connection_node = &nodes[0];
        assert!(connection_node.expanded);
        assert_eq!(labels(&connection_node.children), vec!["public"]);
        assert!(connection_node.children[0].expanded);
        assert_eq!(
            labels(&connection_node.children[0].children),
            vec!["users", "active_users"]
        );
    }

    #[test]
    fn filter_match_keeps_its_whole_subtree() {
        let entry = loaded_entry("local");

        let nodes = build_tree(&[entry], &[], &HashSet::new(), "audit");

        let schema = &nodes[0].children[0];
        assert_eq!(schema.label.as_ref(), "audit");
        assert_eq!(labels(&schema.children), vec!["events"]);
    }

    #[test]
    fn filter_matches_groups_and_ignores_status_rows() {
        let connections = vec![
            ConnectionEntry::new(profile("one", Some("staging"))),
            ConnectionEntry::new(profile("two", None)),
        ];

        let nodes = build_tree(&connections, &[], &HashSet::new(), "stag");
        assert_eq!(labels(&nodes), vec!["staging"]);

        let nodes = build_tree(&connections, &[], &HashSet::new(), "loading");
        assert!(nodes.is_empty());
    }

    #[test]
    fn table_ids_are_unique_across_schemas() {
        let connection = ConnectionId::new();
        assert_ne!(
            table_node_id(connection, Some("a:b"), "c"),
            table_node_id(connection, Some("a"), "b:c")
        );
    }
}
