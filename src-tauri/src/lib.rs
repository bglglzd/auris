mod commands;

use std::sync::Mutex;

use commands::AppState;
use uxo_core::recorder::Recorder;
use uxo_core::storage::Repo;

/// Выбирает рекордер: WASAPI на Windows, CoreAudio + ScreenCaptureKit на
/// macOS, иначе — мок (тишина).
fn build_recorder() -> Box<dyn Recorder> {
    #[cfg(target_os = "windows")]
    {
        Box::new(uxo_core::wasapi_recorder::WasapiRecorder::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(uxo_core::mac_recorder::MacRecorder::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Box::new(uxo_core::recorder::MockRecorder::new(5))
    }
}

/// macOS: ONNX Runtime (диаризация, Parakeet) грузится динамически из
/// `Memiro.app/Contents/Frameworks/libonnxruntime.dylib` — на Intel-Mac
/// статической сборки ORT нет. Путь задаём до первого обращения к ORT.
#[cfg(target_os = "macos")]
fn setup_onnxruntime_path() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(macos_dir) = exe.parent() else { return };
    let candidates = [
        macos_dir.join("../Frameworks/libonnxruntime.dylib"),
        // `tauri dev`: бинарь в target/…, dylib — в src-tauri/macos.
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("macos/libonnxruntime.dylib"),
    ];
    if let Some(p) = candidates.iter().find(|p| p.exists()) {
        std::env::set_var("ORT_DYLIB_PATH", p);
    }
}

/// Переключает запись и сообщает фронтенду событием `recording-changed`.
/// Используется горячей клавишей и пунктом трея.
fn toggle_and_notify<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::{Emitter, Manager};
    let state = app.state::<AppState>();
    match commands::toggle_recording_state(&state) {
        Ok(now_recording) => {
            let _ = app.emit("recording-changed", now_recording);
            use tauri_plugin_notification::NotificationExt;
            let (title, body) = if now_recording {
                ("🔴 Memiro — запись начата", "Идёт запись звонка")
            } else {
                ("✅ Memiro — запись остановлена", "Запись сохранена")
            };
            let _ = app.notification().builder().title(title).body(body).show();
            if now_recording {
                commands::report_recorder_warning(app, &state);
            }
        }
        Err(e) => {
            let _ = app.emit("recording-error", e.to_string());
        }
    }
}

/// Регистрирует глобальную горячую клавишу по умолчанию (Ctrl+Shift+R,
/// на macOS — ⌘⇧R) —
/// работает сразу при старте, до загрузки фронтенда. Фронтенд при загрузке
/// перерегистрирует сохранённое сочетание через `update_hotkey`.
fn setup_global_shortcut(app: &tauri::App) {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
    let gs = app.global_shortcut();
    // Снимаем возможную «висящую» регистрацию (от прошлого инстанса), иначе
    // register() падает «HotKey already registered» и валит весь setup-хук.
    let _ = gs.unregister_all();
    let primary = if cfg!(target_os = "macos") { Modifiers::SUPER } else { Modifiers::CONTROL };
    let shortcut = Shortcut::new(Some(primary | Modifiers::SHIFT), Code::KeyR);
    // Не валим запуск, если не удалось: фронт перерегистрирует через
    // update_hotkey, плюс есть управление из трея.
    if let Err(e) = gs.register(shortcut) {
        eprintln!("setup_global_shortcut: register failed: {e}");
    }
}

/// Меняет глобальную горячую клавишу старт/стоп записи. Снимает все прежние и
/// регистрирует новую из акселератора (напр. "Ctrl+Shift+R"). Пустая строка
/// или None — выключает хоткей. Обработчик (toggle_and_notify) общий для любого
/// зарегистрированного сочетания.
#[tauri::command]
fn update_hotkey(app: tauri::AppHandle, accelerator: Option<String>) -> Result<(), String> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if let Some(acc) = accelerator {
        let acc = acc.trim();
        if !acc.is_empty() {
            let shortcut = Shortcut::from_str(acc)
                .map_err(|e| format!("неверное сочетание «{acc}»: {e}"))?;
            gs.register(shortcut).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Создаёт значок в трее с меню: старт/стоп, открыть окно, выход.
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    let toggle_i = MenuItem::with_id(app, "toggle", "Старт/Стоп записи", true, None::<&str>)?;
    let open_i = MenuItem::with_id(app, "open", "Открыть Memiro", true, None::<&str>)?;
    let bug_i = MenuItem::with_id(app, "bug", "Сообщить об ошибке…", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle_i, &open_i, &bug_i, &quit_i])?;

    // На macOS — монохромный шаблон: строка меню сама красит его под тему.
    #[cfg(target_os = "macos")]
    let builder = TrayIconBuilder::new()
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = TrayIconBuilder::new().icon(app.default_window_icon().unwrap().clone());

    let _tray = builder
        .tooltip("Memiro AI")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "toggle" => toggle_and_notify(app),
            "bug" => {
                use tauri::Emitter;
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit("app-menu", "bug");
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Строка меню macOS по канонам Apple: меню приложения (О программе,
/// Настройки ⌘,, Проверить обновления, Скрыть, Завершить), Файл, Правка
/// (без неё не работают ⌘C/⌘V в полях), Вид, Окно. Пункты, которые ведёт
/// фронтенд, приходят ему событием `app-menu` с id пункта.
#[cfg(target_os = "macos")]
fn setup_mac_menu(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    use tauri::Emitter;

    let about = AboutMetadata {
        name: Some("Memiro AI".into()),
        version: Some(app.package_info().version.to_string()),
        comments: Some("Память ваших встреч — локальная запись и расшифровка".into()),
        website: Some("https://github.com/bglglzd/auris".into()),
        website_label: Some("github.com/bglglzd/auris".into()),
        ..Default::default()
    };
    let settings = MenuItemBuilder::with_id("settings", "Настройки…")
        .accelerator("Cmd+,")
        .build(app)?;
    let updates = MenuItemBuilder::with_id("updates", "Проверить обновления…").build(app)?;
    let app_menu = SubmenuBuilder::new(app, "Memiro AI")
        .about(Some(about))
        .separator()
        .item(&settings)
        .item(&updates)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    // Без акселератора: ⌘⇧R уже занят глобальным хоткеем (иначе двойной
    // переключатель, когда окно в фокусе).
    let record = MenuItemBuilder::with_id("record", "Начать или остановить запись").build(app)?;
    let solo = MenuItemBuilder::with_id("solo", "Заметка · я один").build(app)?;
    let import = MenuItemBuilder::with_id("import", "Импорт записи…")
        .accelerator("Cmd+O")
        .build(app)?;
    let file_menu = SubmenuBuilder::new(app, "Файл")
        .item(&record)
        .item(&solo)
        .separator()
        .item(&import)
        .separator()
        .close_window()
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Правка")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let find = MenuItemBuilder::with_id("find", "Поиск встреч")
        .accelerator("Cmd+F")
        .build(app)?;
    let theme = MenuItemBuilder::with_id("theme", "Светлая / тёмная тема")
        .accelerator("Cmd+Shift+L")
        .build(app)?;
    let view_menu = SubmenuBuilder::new(app, "Вид")
        .item(&find)
        .item(&theme)
        .separator()
        .fullscreen()
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "Окно")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let bug = MenuItemBuilder::with_id("bug", "Сообщить об ошибке…").build(app)?;
    let help_menu = SubmenuBuilder::new(app, "Справка").item(&bug).build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu, &help_menu])
        .build()?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| match event.id.as_ref() {
        "record" => toggle_and_notify(app),
        id @ ("settings" | "updates" | "solo" | "import" | "find" | "theme" | "bug") => {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            let _ = app.emit("app-menu", id);
        }
        _ => {}
    });
    Ok(())
}

const AUTORECORD_POLL_MS: u64 = 2500;

/// Фоновый монитор авто-записи: каждые ~2.5с проверяет аудио-сессии выбранных
/// приложений (`AppState.autorecord`) и стартует/стопит запись. Останавливает
/// только то, что начал сам (`auto_active`), не трогая ручную запись.
///
/// Чтобы не записывать короткие звуки уведомлений (Telegram «дзынь» ~2 с):
/// 1) старт только если звонок держится непрерывно ≥ `start_delay_secs`
///    (несколько опросов подряд — `active_streak`);
/// 2) на авто-стопе запись короче `min_keep_secs` удаляется как мусорный огрызок.
fn spawn_autorecord_monitor<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    use tauri::Manager;
    std::thread::spawn(move || {
        let mut auto_active = false;
        // Сколько опросов подряд звонок был активен (для задержки старта).
        let mut active_streak: u32 = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(AUTORECORD_POLL_MS));
            let (enabled, processes, auto_stop, start_delay_secs, min_keep_secs) = {
                // Привязываем State к переменной: иначе временное значение из
                // app.state() дропается до использования гарда (E0716).
                let state = app.state::<AppState>();
                let cfg = state.autorecord.lock().unwrap();
                (
                    cfg.enabled,
                    cfg.processes.clone(),
                    cfg.auto_stop,
                    cfg.start_delay_secs,
                    cfg.min_keep_secs,
                )
            };
            if !enabled || processes.is_empty() {
                auto_active = false;
                active_streak = 0;
                continue;
            }
            let recording = {
                let state = app.state::<AppState>();
                let active = state.active.lock().unwrap();
                active.is_some()
            };
            // Ручная остановка извне — сбрасываем флаг авто-записи.
            if !recording {
                auto_active = false;
            }
            let call = uxo_core::call_detector::any_active_call(&processes);
            active_streak = if call { active_streak.saturating_add(1) } else { 0 };

            // Сколько опросов подряд требуется до старта (округление вверх; >=1).
            let required = (((start_delay_secs as u64 * 1000) + AUTORECORD_POLL_MS - 1)
                / AUTORECORD_POLL_MS)
                .max(1) as u32;

            if call && !recording && active_streak >= required {
                toggle_and_notify(&app);
                auto_active = true;
            } else if !call && recording && auto_active && auto_stop {
                auto_stop_and_maybe_discard(&app, min_keep_secs);
                auto_active = false;
                active_streak = 0;
            }
        }
    });
}

/// Авто-стоп записи: останавливает и, если получившаяся запись короче
/// `min_keep_secs`, удаляет её как мусорный огрызок (звук уведомления).
fn auto_stop_and_maybe_discard<R: tauri::Runtime>(app: &tauri::AppHandle<R>, min_keep_secs: u32) {
    use tauri::{Emitter, Manager};
    let state = app.state::<AppState>();
    match commands::stop_active_recording(&state) {
        Ok(Some(meeting)) => {
            let _ = app.emit("recording-changed", false);
            if min_keep_secs > 0 && meeting.duration_secs < min_keep_secs as u64 {
                let _ = commands::discard_meeting(&state, &meeting.id);
                commands::flog(
                    &state.data_root,
                    &format!(
                        "autorecord: discarded short clip {}s (id={})",
                        meeting.duration_secs, meeting.id
                    ),
                );
                let _ = app.emit("recording-changed", false);
            } else {
                use tauri_plugin_notification::NotificationExt;
                let _ = app
                    .notification()
                    .builder()
                    .title("✅ Memiro — запись остановлена")
                    .body("Запись сохранена")
                    .show();
            }
        }
        Ok(None) => {}
        Err(e) => {
            let _ = app.emit("recording-error", e.to_string());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    setup_onnxruntime_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        toggle_and_notify(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            use tauri::Manager;
            let data_root = app.path().app_data_dir().expect("no app data dir");
            std::fs::create_dir_all(&data_root).expect("cannot create data dir");
            // Panic-hook пишет в файл лога — переживает нативный краш.
            let log_path = data_root.join("3uxo.log");
            std::panic::set_hook(Box::new(move |info| {
                use std::io::Write;
                let line = format!(
                    "[panic] {}\n  location: {:?}\n",
                    info,
                    info.location()
                );
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    let _ = f.write_all(line.as_bytes());
                }
            }));

            let db_path = data_root.join("3uxo.db");
            let repo = Repo::open(&db_path).expect("cannot open db");

            // Восстанавливаем записи, оборванные аварийным завершением: склеиваем
            // осиротевшие сегменты в единый файл (см. фичу «склейка фрагментов»).
            match uxo_core::service::recover_orphan_recordings(
                &repo,
                &data_root,
                chrono::Utc::now().to_rfc3339(),
            ) {
                Ok(n) if n > 0 => {
                    commands::flog(&data_root, &format!("recovered {n} orphan recording(s)"))
                }
                Ok(_) => {}
                Err(e) => commands::flog(&data_root, &format!("recover orphan failed: {e}")),
            }

            app.manage(AppState {
                data_root,
                repo: Mutex::new(repo),
                recorder: build_recorder(),
                active: Mutex::new(None),
                autorecord: std::sync::Arc::new(Mutex::new(
                    commands::AutoRecordCfg::default(),
                )),
            });

            setup_global_shortcut(app);
            setup_tray(app)?;
            #[cfg(target_os = "macos")]
            setup_mac_menu(app)?;
            spawn_autorecord_monitor(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_recording,
            commands::stop_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::recording_state,
            commands::recording_levels,
            commands::import_recording,
            commands::list_meetings,
            commands::get_meeting,
            commands::delete_meeting,
            commands::track_path,
            commands::is_recording,
            commands::transcribe,
            commands::get_transcript,
            commands::save_transcript,
            commands::waveform,
            commands::audio_edit_state,
            commands::apply_audio_edit,
            commands::revert_audio_edit,
            commands::save_report,
            commands::suggest_metadata,
            commands::summarize,
            commands::get_summary,
            commands::literary_text,
            commands::get_literary,
            commands::brief_summary,
            commands::get_brief,
            commands::analyze,
            commands::get_analysis,
            commands::ask,
            commands::update_meeting_meta,
            commands::save_text_file,
            commands::export_audio,
            commands::get_backend_log,
            commands::set_autorecord,
            commands::recluster_speakers,
            commands::has_voice_analysis,
            commands::models_status,
            commands::download_model,
            commands::delete_model,
            commands::generate_report,
            commands::get_reports,
            commands::suggest_meta,
            commands::ask_named,
            commands::save_binary_file,
            commands::update_meeting_notes,
            commands::ai_check,
            commands::platform,
            commands::system_info,
            commands::system_audio_access,
            commands::open_privacy_settings,
            update_hotkey,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
