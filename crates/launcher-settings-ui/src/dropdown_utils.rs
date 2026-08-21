use gtk4 as gtk;
use gtk::prelude::*;

pub fn dropdown_active_id(dropdown: &gtk::DropDown) -> Option<String> {
    dropdown
        .selected_item()
        .and_downcast::<gtk::StringObject>()
        .map(|s| s.string().to_string())
}

pub fn dropdown_set_active_id(dropdown: &gtk::DropDown, id: &str) -> bool {
    if let Some(model) = dropdown.model() {
        for i in 0..model.n_items() {
            if let Some(obj) = model.item(i).and_downcast::<gtk::StringObject>() {
                if obj.string() == id {
                    dropdown.set_selected(i);
                    return true;
                }
            }
        }
    }
    dropdown.set_selected(gtk::INVALID_LIST_POSITION);
    false
}

pub fn dropdown_append(dropdown: &gtk::DropDown, id_and_text: &str) {
    if let Some(list) = dropdown.model().and_downcast::<gtk::StringList>() {
        list.append(id_and_text);
    }
}

pub fn dropdown_remove_all(dropdown: &gtk::DropDown) {
    if let Some(list) = dropdown.model().and_downcast::<gtk::StringList>() {
        list.splice(0, list.n_items(), &[]);
    }
}

pub fn dropdown_remove_id(dropdown: &gtk::DropDown, id: &str) {
    if let Some(list) = dropdown.model().and_downcast::<gtk::StringList>() {
        for i in 0..list.n_items() {
            if let Some(obj) = list.item(i).and_downcast::<gtk::StringObject>() {
                if obj.string() == id {
                    list.splice(i, 1, &[]);
                    return;
                }
            }
        }
    }
}

pub fn dropdown_has_id(dropdown: &gtk::DropDown, id: &str) -> bool {
    if let Some(model) = dropdown.model() {
        for i in 0..model.n_items() {
            if let Some(obj) = model.item(i).and_downcast::<gtk::StringObject>() {
                if obj.string() == id {
                    return true;
                }
            }
        }
    }
    false
}

pub fn create_dropdown() -> gtk::DropDown {
    let list = gtk::StringList::new(&[]);
    gtk::DropDown::new(Some(list), gtk::Expression::NONE)
}
