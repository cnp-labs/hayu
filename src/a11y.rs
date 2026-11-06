//! AT-SPI exposure for the launcher.
//!
//! Scope of this spike: the launcher window, its search field, and the result
//! rows. The bar, actions and the IME preedit stay out until the render
//! interface migration, because the launcher is the only surface a screen
//! reader has to reach today.

use std::sync::{Arc, Mutex};

use accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId, Role,
    TreeId, TreeInfo, TreeUpdate,
};
use accesskit_unix::Adapter;

use crate::launcher::Launcher;

const ROOT: NodeId = NodeId(1);
const INPUT: NodeId = NodeId(2);
const LIST: NodeId = NodeId(3);
const TEXT: NodeId = NodeId(4);
const ITEM: u64 = 16;

pub struct A11y {
    adapter: Adapter,
    /// Latest tree, so an assistive technology that connects while the
    /// launcher is already open gets real content instead of the adapter's
    /// placeholder. `update_if_active` only runs on our own changes.
    last: Arc<Mutex<Option<TreeUpdate>>>,
}

impl A11y {
    pub fn new() -> Self {
        let last = Arc::new(Mutex::new(None));
        Self {
            adapter: Adapter::new(Initial(Arc::clone(&last)), NoAction, NoDeactivation),
            last,
        }
    }

    /// Publish the current launcher state. Cheap enough to call on every draw.
    pub fn sync(&mut self, launcher: &Launcher) {
        let update = tree(launcher);
        if let Ok(mut last) = self.last.lock() {
            *last = Some(update.clone());
        }
        self.adapter.update_window_focus_state(launcher.is_open());
        self.adapter.update_if_active(|| update);
    }
}

fn tree(launcher: &Launcher) -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.set_label("Hayu launcher");
    let mut nodes = Vec::new();
    let mut focus = ROOT;
    if launcher.is_open() {
        let mut input = Node::new(Role::TextInput);
        input.set_label("Search applications");
        // AT-SPI only exposes the Text interface when the input owns a text
        // run with character lengths; a bare `value` stays unreadable.
        let query = launcher.query();
        let mut text = Node::new(Role::TextRun);
        text.set_value(query.to_string());
        text.set_character_lengths(
            query
                .chars()
                .map(|c| c.len_utf8() as u8)
                .collect::<Vec<_>>(),
        );
        input.set_children([TEXT]);

        let results = launcher.results();
        let mut list = Node::new(Role::List);
        list.set_label("Results");
        let ids: Vec<NodeId> = (0..results.len())
            .map(|index| NodeId(ITEM + index as u64))
            .collect();
        list.set_children(ids.clone());
        for (index, entry) in results.iter().enumerate() {
            let mut item = Node::new(Role::ListItem);
            item.set_label(entry.name.clone());
            item.set_description(entry.desktop_id.clone());
            item.set_position_in_set(index + 1);
            item.set_size_of_set(results.len());
            if index == launcher.selected() {
                item.set_selected(true);
            }
            nodes.push((NodeId(ITEM + index as u64), item));
        }

        root.set_children([INPUT, LIST]);
        nodes.push((INPUT, input));
        nodes.push((TEXT, text));
        nodes.push((LIST, list));
        focus = INPUT;
    } else {
        // No API removes a tree root, so a closed launcher stays as a hidden
        // window instead of a stale list a screen reader could still reach.
        root.set_hidden();
    }
    nodes.push((ROOT, root));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(ROOT)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

struct Initial(Arc<Mutex<Option<TreeUpdate>>>);

impl ActivationHandler for Initial {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0.lock().ok().and_then(|last| last.clone())
    }
}

/// Actions are out of scope for this spike: the tree is read-only.
struct NoAction;

impl ActionHandler for NoAction {
    fn do_action(&mut self, _request: ActionRequest) {}
}

struct NoDeactivation;

impl DeactivationHandler for NoDeactivation {
    fn deactivate_accessibility(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Entry;

    fn entry(id: &str, name: &str) -> Entry {
        Entry {
            desktop_id: id.into(),
            name: name.into(),
            exec: vec!["app".into()],
            terminal: false,
        }
    }

    #[test]
    fn open_tree_exposes_search_field_and_selection() {
        let entries = vec![
            entry("firefox.desktop", "Firefox"),
            entry("files.desktop", "Files"),
        ];
        let mut launcher = Launcher::new(entries);
        launcher.begin_open();
        launcher.insert_text("fi");
        launcher.key(&smithay_client_toolkit::seat::keyboard::KeyEvent {
            time: 0,
            raw_code: 0,
            keysym: xkbcommon::xkb::Keysym::new(xkbcommon::xkb::keysyms::KEY_Down),
            utf8: None,
        });
        let update = tree(&launcher);
        assert_eq!(update.focus, INPUT);
        let by_id = |id: NodeId| {
            update
                .nodes
                .iter()
                .find(|(node_id, _)| *node_id == id)
                .map(|(_, node)| node)
                .unwrap()
        };
        assert_eq!(by_id(INPUT).role(), Role::TextInput);
        assert_eq!(by_id(INPUT).children(), [TEXT]);
        assert_eq!(by_id(TEXT).value(), Some("fi"));
        assert_eq!(by_id(TEXT).character_lengths(), [1, 1]);
        assert!(!launcher.results().is_empty());
        let selected = by_id(NodeId(ITEM + launcher.selected() as u64));
        assert_eq!(selected.is_selected(), Some(true));
        assert_eq!(selected.role(), Role::ListItem);
        assert_eq!(by_id(ROOT).role(), Role::Window);
    }

    #[test]
    fn closed_tree_hides_the_window() {
        let launcher = Launcher::new(vec![entry("a.desktop", "Alpha")]);
        let update = tree(&launcher);
        let root = update
            .nodes
            .iter()
            .find(|(id, _)| *id == ROOT)
            .map(|(_, node)| node)
            .unwrap();
        assert!(root.is_hidden());
        assert_eq!(update.focus, ROOT);
    }
}
