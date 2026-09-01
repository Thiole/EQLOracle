//! why: throwaway debug -- print exactly what the clipboard holds, since
//! this box has no xclip/wl-paste. run: cargo run -p eqlp-app --example clipboard_check
fn main() {
    gtk::init().expect("gtk init");
    let cb = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    match cb.wait_for_text() {
        Some(t) => {
            println!("clipboard text ({} chars):", t.chars().count());
            println!("{t}");
            println!("-- bytes: {:?}", &t.as_bytes()[..t.len().min(120)]);
        }
        None => println!("clipboard holds no text (empty or non-text target)"),
    }
}
