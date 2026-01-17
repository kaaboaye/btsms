use gtk4::prelude::*;
use gtk4::ListBox;

/// Clears all children from a ListBox.
/// This pattern is used throughout the GUI code when refreshing lists.
pub fn clear_list_box(list: &ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clear_list_box_exists() {
        // Verify the function signature compiles correctly
        let _: fn(&ListBox) = clear_list_box;
    }
}
