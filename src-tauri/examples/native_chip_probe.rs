use gtk::prelude::*;

fn main() {
    gtk::init().expect("GTK init failed");

    let window = gtk::Window::new(gtk::WindowType::Toplevel);

    window.set_title("StickyFlow Native Chip Probe");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_accept_focus(false);
    window.set_focus_on_map(false);

    window.set_default_size(90, 26);
    window.resize(90, 26);

    let label = gtk::Label::new(Some("⋮  test  ›"));
    label.set_xalign(0.5);
    label.set_yalign(0.5);

    let provider = gtk::CssProvider::new();

    provider
        .load_from_data(
            b"
            window {
                background: #fff1a8;
            }

            label {
                color: #29261e;
                font-size: 11px;
                font-weight: 600;
                padding: 0px 5px;
            }
            ",
        )
        .expect("CSS load failed");

    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    window.add(&label);

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        gtk::glib::Propagation::Proceed
    });

    window.show_all();

    // Apply exact size again after GTK has realized the widgets.
    window.resize(90, 26);

    gtk::main();
}
