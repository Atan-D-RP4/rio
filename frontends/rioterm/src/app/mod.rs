mod menu;
mod route;

use rio_backend::event::{EventProxy, RioEventType, EventPayload};
use crate::event::RioEvent;
use crate::ime::Preedit;
use crate::routes::RoutePath;
// use crate::scheduler::{Scheduler, TimerId, Topic};
use rio_backend::error::RioError;
use wa::event_loop::{EventLoop, EventLoopProxy};
use rio_backend::sugarloaf::font::FontLibrary;
use route::Route;
use std::error::Error;
use std::rc::Rc;
use wa::*;

struct Router {
    config: Rc<rio_backend::config::Config>,
    route: Option<Route>,
    event_loop_proxy: EventLoopProxy<EventPayload>,
    font_library: FontLibrary,
    #[cfg(target_os = "macos")]
    tab_group: Option<u64>,
}

pub fn create_window(
    event_loop_proxy: EventLoopProxy<EventPayload>,
    config: &Rc<rio_backend::config::Config>,
    config_error: &Option<rio_backend::config::ConfigError>,
    font_library: FontLibrary,
    tab_group: Option<u64>,
) -> Result<Window, Box<dyn std::error::Error>> {
    // if config_error.is_some() {
    //     superloop.send_event(
    //         RioEvent::ReportToAssistant(RioError::configuration_not_found()),
    //         0,
    //     );
    // }

    let router = Router {
        config: config.clone(),
        route: None,
        event_loop_proxy,
        font_library,
        tab_group,
    };

    let hide_toolbar_buttons = config.window.decorations
        == rio_backend::config::window::Decorations::Buttonless
        || config.window.decorations
            == rio_backend::config::window::Decorations::Disabled;

    #[cfg(target_os = "macos")]
    let tab_identifier = if tab_group.is_some() {
        Some(format!("tab-group-{}", tab_group.unwrap()))
    } else {
        None
    };

    let wa_conf = conf::Conf {
        window_title: String::from("~"),
        window_width: config.window.width,
        window_height: config.window.height,
        fullscreen: config.window.is_fullscreen(),
        transparency: config.window.background_opacity < 1.,
        blur: config.window.blur,
        hide_toolbar: !config.navigation.is_native(),
        hide_toolbar_buttons,
        #[cfg(target_os = "macos")]
        tab_identifier,
        ..Default::default()
    };

    futures::executor::block_on(Window::new_window(wa_conf, || Box::new(router)))
}

impl EventHandler for Router {
    fn init(
        &mut self,
        id: u16,
        raw_window_handle: raw_window_handle::RawWindowHandle,
        raw_display_handle: raw_window_handle::RawDisplayHandle,
        width: i32,
        height: i32,
        scale_factor: f32,
        open_url: &str,
    ) {
        let event_proxy = EventProxy::new(self.event_loop_proxy.clone());
        let initial_route = Route::new(
            id,
            event_proxy,
            raw_window_handle,
            raw_display_handle,
            self.config.clone(),
            self.font_library.clone(),
            (width, height, scale_factor),
            open_url,
        )
        .expect("Expected window to be created");
        self.route = Some(initial_route);
    }

    fn focus_event(&mut self, focused: bool) {
        if let Some(current) = &mut self.route {
            current.is_focused = focused;
            current.on_focus_change(focused);
        }
    }

    fn ime_event(&mut self, ime_state: ImeState) {
        if let Some(current) = &mut self.route {
            if current.path != RoutePath::Terminal {
                return;
            }

            match ime_state {
                ImeState::Commit(text) => {
                    // Don't use bracketed paste for single char input.
                    current.paste(&text, text.chars().count() > 1);
                }
                ImeState::Preedit(text, cursor_offset) => {
                    let preedit = if text.is_empty() {
                        None
                    } else {
                        Some(Preedit::new(text, cursor_offset.map(|offset| offset.0)))
                    };

                    if current.ime.preedit() != preedit.as_ref() {
                        current.ime.set_preedit(preedit);
                        current.render();
                    }
                }
                ImeState::Enabled => {
                    current.ime.set_enabled(true);
                }
                ImeState::Disabled => {
                    current.ime.set_enabled(false);
                }
            }
        }
    }

    fn modifiers_event(&mut self, keycode: KeyCode, mods: ModifiersState) {
        if let Some(current) = &mut self.route {
            current.set_modifiers(mods);

            if (keycode == KeyCode::LeftSuper || keycode == KeyCode::RightSuper)
                && current.search_nearest_hyperlink_from_pos()
            {
                window::set_mouse_cursor(current.id, wa::CursorIcon::Pointer);
                current.render();
            }
        }
    }

    fn key_down_event(
        &mut self,
        keycode: KeyCode,
        repeat: bool,
        character: Option<smol_str::SmolStr>,
    ) {
        // FIX: Tab isn't being captured whenever other key is holding
        if let Some(current) = &mut self.route {
            if current.has_key_wait(keycode) {
                return;
            }

            current.process_key_event(keycode, true, repeat, character);
        }
    }
    fn key_up_event(&mut self, keycode: KeyCode) {
        if let Some(current) = &mut self.route {
            if current.has_key_wait(keycode) {
                return;
            }

            current.process_key_event(keycode, false, false, None);
            current.render();
        }
    }
    fn mouse_motion_event(&mut self, x: f32, y: f32) {
        if let Some(current) = &mut self.route {
            if current.path != RoutePath::Terminal {
                return;
            }

            if self.config.hide_cursor_when_typing {
                window::show_mouse(current.id, true);
            }

            if let Some(cursor) = current.process_motion_event(x, y) {
                window::set_mouse_cursor(current.id, cursor);
            }
        }
    }
    fn appearance_change_event(&mut self, appearance: Appearance) {
        if let Some(current) = &mut self.route {
            current.update_config(&self.config, appearance);
        }
    }
    fn touch_event(&mut self, phase: TouchPhase, _id: u64, _x: f32, _y: f32) {
        if phase == TouchPhase::Started {
            if let Some(current) = &mut self.route {
                current.mouse.accumulated_scroll = Default::default();
            }
        }
    }
    fn open_file_event(&mut self, filepath: String) {
        if let Some(current) = &mut self.route {
            current.paste(&filepath, true);
        }
    }
    // fn open_url_event(&mut self, _url: &str) {
    // if let Some(current) = &mut self.route {
    //     current.paste(&url, true);
    //     current.render();
    // }
    // }
    fn mouse_wheel_event(&mut self, mut x: f32, mut y: f32) {
        if let Some(current) = &mut self.route {
            if current.path != RoutePath::Terminal {
                return;
            }

            if self.config.hide_cursor_when_typing {
                window::show_mouse(current.id, true);
            }

            // match delta {
            //     MouseScrollDelta::LineDelta(columns, lines) => {
            //         let new_scroll_px_x = columns
            //             * route.window.screen.sugarloaf.layout.font_size;
            //         let new_scroll_px_y = lines
            //             * route.window.screen.sugarloaf.layout.font_size;
            //         route.window.screen.scroll(
            //             new_scroll_px_x as f64,
            //             new_scroll_px_y as f64,
            //         );
            //     }

            // When the angle between (x, 0) and (x, y) is lower than ~25 degrees
            // (cosine is larger that 0.9) we consider this scrolling as horizontal.
            if x.abs() / x.hypot(y) > 0.9 {
                y = 0.;
            } else {
                x = 0.;
            }

            current.scroll(x.into(), y.into());
            // current.render();
        }
    }
    fn mouse_button_down_event(&mut self, button: MouseButton, x: f32, y: f32) {
        if let Some(current) = &mut self.route {
            if current.path != RoutePath::Terminal {
                return;
            }

            if self.config.hide_cursor_when_typing {
                window::show_mouse(current.id, true);
            }

            current.process_mouse(button, x, y, true);
        }
    }
    fn mouse_button_up_event(&mut self, button: MouseButton, x: f32, y: f32) {
        if let Some(current) = &mut self.route {
            if current.path != RoutePath::Terminal {
                return;
            }

            if self.config.hide_cursor_when_typing {
                window::show_mouse(current.id, true);
            }

            current.process_mouse(button, x, y, false);
        }
    }
    fn resize_event(&mut self, w: i32, h: i32, scale_factor: f32, rescale: bool) {
        if let Some(current) = &mut self.route {
            if rescale {
                current.sugarloaf.rescale(scale_factor);
                current
                    .sugarloaf
                    .resize(w.try_into().unwrap(), h.try_into().unwrap());
            } else {
                current
                    .sugarloaf
                    .resize(w.try_into().unwrap(), h.try_into().unwrap());
            }
            current.resize_all_contexts();
        }
    }
    fn quit_requested_event(&mut self) {
        // window::cancel_quit(self.id);
    }
    fn files_dragged_event(
        &mut self,
        _filepaths: Vec<std::path::PathBuf>,
        drag_state: DragState,
    ) {
        if let Some(current) = &mut self.route {
            match drag_state {
                DragState::Entered => {
                    current.state.decrease_foreground_opacity(0.3);
                    current.render();
                }
                DragState::Exited => {
                    current.state.increase_foreground_opacity(0.3);
                    current.render();
                }
            }
        }
    }
    fn files_dropped_event(&mut self, filepaths: Vec<std::path::PathBuf>) {
        if filepaths.is_empty() {
            return;
        }

        if let Some(current) = &mut self.route {
            if current.path != RoutePath::Terminal {
                return;
            }

            let mut dropped_files = String::from("");
            for filepath in filepaths {
                dropped_files.push_str(&(filepath.to_string_lossy().to_string() + " "));
            }

            if !dropped_files.is_empty() {
                current.paste(&dropped_files, true);
            }
        }
    }
}

struct AppInstance {
    config: Rc<rio_backend::config::Config>,
    font_library: FontLibrary,
    event_loop: EventLoop<rio_backend::event::EventPayload>,
    #[cfg(target_os = "macos")]
    last_tab_group: Option<u64>,
}

impl AppInstance {
    fn new(config: rio_backend::config::Config) -> Self {
        let font_library = FontLibrary::new(config.fonts.to_owned());
        // let mut sugarloaf_errors = None;

        // let (font_library, fonts_not_found) = loader;

        // if !fonts_not_found.is_empty() {
        //     sugarloaf_errors = Some(SugarloafErrors { fonts_not_found });
        // }

        let config = Rc::new(config);
        let event_loop = EventLoop::<rio_backend::event::EventPayload>::build().expect("expected event loop to be created");
        Self {
            event_loop,
            font_library,
            config,
            #[cfg(target_os = "macos")]
            last_tab_group: None,
        }
    }
}

impl AppHandler for AppInstance {
    fn create_window(&self) {
        let _ = create_window(self.event_loop.create_proxy(), &self.config, &None, self.font_library.clone(), None);
    }

    fn create_tab(&self, open_file_url: Option<&str>) {
        if let Ok(window) = create_window(
            self.event_loop.create_proxy(),
            &self.config,
            &None,
            self.font_library.clone(),
            self.last_tab_group,
        ) {
            if let Some(file_url) = open_file_url {
                wa::window::open_url(window.id, file_url);
            }
        }
    }

    #[inline]
    fn process(&mut self) {
        if let Ok(event) = self.event_loop.receiver.try_recv() {
            let window_id = event.window_id;
            match event.payload {
                RioEventType::Rio(RioEvent::CloseWindow) => {
                    // TODO
                }
                RioEventType::Rio(RioEvent::CreateWindow) => {
                    #[cfg(target_os = "macos")]
                    let new_tab_group = if self.config.navigation.is_native() {
                        self.tab_group
                            .map(|current_tab_group| current_tab_group + 1)
                    } else {
                        None
                    };

                    let _ = create_window(
                        self.event_loop.create_proxy(),
                        &self.config,
                        &None,
                        self.font_library.clone(),
                        new_tab_group,
                    );
                }
                #[cfg(target_os = "macos")]
                RioEventType::Rio(RioEvent::CreateNativeTab(_)) => {
                    let _ = create_window(
                        self.event_loop.create_proxy(),
                        &self.config,
                        &None,
                        self.font_library.clone(),
                        self.tab_group,
                    );
                }
                RioEventType::Rio(RioEvent::UpdateConfig) => {
                    let (config, config_error) =
                        match rio_backend::config::Config::try_load() {
                            Ok(config) => (config, None),
                            Err(error) => {
                                (rio_backend::config::Config::default(), Some(error))
                            }
                        };

                    self.config = config.into();
                    let appearance = wa::window::get_appearance();

                    if let Some(current) = &mut self.route {
                        if let Some(error) = &config_error {
                            current.report_error(&error.to_owned().into());
                        } else {
                            current.clear_assistant_errors();
                        }

                        current.update_config(&self.config, appearance);
                    }
                }
                RioEventType::Rio(RioEvent::TitleWithSubtitle(title, subtitle)) => {
                    if let Some(current) = &mut self.route {
                        window::set_window_title(current.id, title, subtitle);
                    }
                }
                RioEventType::Rio(RioEvent::MouseCursorDirty) => {
                    if let Some(current) = &mut self.route {
                        current.mouse.accumulated_scroll =
                            crate::mouse::AccumulatedScroll::default();
                    }
                }
                RioEventType::Rio(RioEvent::Scroll(scroll)) => {
                    if let Some(current) = &mut self.route {
                        let mut terminal = current.ctx.current().terminal.lock();
                        terminal.scroll_display(scroll);
                        drop(terminal);
                    }
                }
                RioEventType::Rio(RioEvent::Quit) => {
                    window::request_quit();
                }
                RioEventType::Rio(RioEvent::ClipboardLoad(clipboard_type, format)) => {
                    if let Some(current) = &mut self.route {
                        // if route.window.is_focused {
                        let text = format(current.clipboard_get(clipboard_type).as_str());
                        current
                            .ctx
                            .current_mut()
                            .messenger
                            .send_bytes(text.into_bytes());
                        // }
                    }
                }
                RioEventType::Rio(RioEvent::ClipboardStore(clipboard_type, content)) => {
                    if let Some(current) = &mut self.route {
                        // if current.is_focused {
                        current.clipboard_store(clipboard_type, content);
                        // }
                    }
                }
                RioEventType::Rio(RioEvent::PtyWrite(text)) => {
                    if let Some(current) = &mut self.route {
                        current
                            .ctx
                            .current_mut()
                            .messenger
                            .send_bytes(text.into_bytes());
                    }
                }
                RioEventType::Rio(RioEvent::ReportToAssistant(error)) => {
                    if let Some(current) = &mut self.route {
                        current.report_error(&error);
                    }
                }
                RioEventType::Rio(RioEvent::UpdateGraphicLibrary) => {
                    if let Some(current) = &mut self.route {
                        let mut terminal = current.ctx.current().terminal.lock();
                        let graphics = terminal.graphics_take_queues();
                        if let Some(graphic_queues) = graphics {
                            let renderer = &mut current.sugarloaf;
                            for graphic_data in graphic_queues.pending {
                                renderer.add_graphic(graphic_data);
                            }

                            for graphic_data in graphic_queues.remove_queue {
                                renderer.remove_graphic(&graphic_data);
                            }
                        }
                    }
                }
                // RioEventType::Rio(RioEvent::ScheduleRender(millis) => {
                //     let timer_id = TimerId::new(Topic::Render, 0);
                //     let event = EventPayload::new(RioEventType::Rio(RioEvent::Render, self.current);

                //     if !self.scheduler.scheduled(timer_id) {
                //         self.scheduler.schedule(
                //             event,
                //             Duration::from_millis(millis),
                //             false,
                //             timer_id,
                //         );
                //     }
                // }
                RioEventType::Rio(RioEvent::Noop) => {}
                _ => {}
            };
        }
    }

    // This is executed only in the initialization of App
    fn start(&mut self) {
        self.last_tab_group = if self.config.navigation.is_native() {
            Some(0)
        } else {
            None
        };

        let _ = create_window(
            self.event_loop.create_proxy(),
            &self.config,
            &None,
            self.font_library.clone(),
            self.last_tab_group,
        );
    }
}

#[inline]
pub async fn run(
    config: rio_backend::config::Config,
    _config_error: Option<rio_backend::config::ConfigError>,
) -> Result<(), Box<dyn Error>> {
    // let superloop = Superloop::new();
    let application_instance = AppInstance::new(config);
    // let _ = crate::watcher::configuration_file_updates(superloop.clone());

    // let scheduler = Scheduler::new(superloop.clone());

    App::start(|| Box::new(application_instance));
    menu::create_menu();
    App::run();
    Ok(())
}
