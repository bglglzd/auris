//! Звук собеседников на macOS 14.2+ через Core Audio process tap.
//!
//! Вместо ScreenCaptureKit («Запись экрана и системного звука», перезапуск
//! приложения, ежемесячное напоминание в macOS 15) — отдельное, более мягкое
//! разрешение «Только запись системного звука». Экран не захватывается вовсе.
//!
//! Схема (как в примере Apple «Capturing system audio with Core Audio taps»):
//! `CATapDescription` (моно-сведение всех процессов, кроме самого Memiro) →
//! `AudioHardwareCreateProcessTap` → приватное агрегатное устройство с этим
//! tap → IOProc получает float32 на частоте выхода (обычно 48 кГц) →
//! `TrackSink` (оконный sinc → 16 кГц моно WAV).
//!
//! Функции tap появились только в 14.2, поэтому берутся через `dlsym` во время
//! работы: на macOS 13–14.1 модуль просто сообщает «не поддерживается», и
//! запись идёт прежним путём (ScreenCaptureKit).
//!
//! Статус разрешения публичным API не узнать — используется `TCCAccessPreflight`
//! из системного TCC.framework (так делают и примеры Apple-разработчиков); если
//! его нет — статус «неизвестен», запись всё равно работает.

use std::ffi::{c_void, CStr};
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSString, NSUUID};

use crate::audio::TrackSink;

type Sink = Arc<Mutex<TrackSink>>;

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioHardwareCreateAggregateDevice(description: *const c_void, out: *mut u32) -> i32;
    fn AudioHardwareDestroyAggregateDevice(device: u32) -> i32;
    fn AudioDeviceCreateIOProcID(
        device: u32,
        proc_: IoProc,
        client: *mut c_void,
        out: *mut Option<IoProc>,
    ) -> i32;
    fn AudioDeviceDestroyIOProcID(device: u32, id: Option<IoProc>) -> i32;
    fn AudioDeviceStart(device: u32, id: Option<IoProc>) -> i32;
    fn AudioDeviceStop(device: u32, id: Option<IoProc>) -> i32;
}

// AVCaptureDevice (статус микрофона) живёт в AVFoundation.
#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

type IoProc = unsafe extern "C" fn(
    device: u32,
    now: *const c_void,
    input: *const AudioBufferList,
    input_time: *const c_void,
    output: *mut AudioBufferList,
    output_time: *const c_void,
    client: *mut c_void,
) -> i32;

type CreateTapFn = unsafe extern "C" fn(*mut AnyObject, *mut u32) -> i32;
type DestroyTapFn = unsafe extern "C" fn(u32) -> i32;

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct StreamDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}
const SYSTEM_OBJECT: u32 = 1;
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SEL_DEFAULT_SYSTEM_OUTPUT: u32 = fourcc(b"sOut");
const SEL_PID_TO_PROCESS: u32 = fourcc(b"id2p");
const SEL_DEVICE_UID: u32 = fourcc(b"uid ");
const SEL_TAP_FORMAT: u32 = fourcc(b"tfmt");
const SEL_NOMINAL_RATE: u32 = fourcc(b"nsrt");
const FORMAT_LPCM: u32 = fourcc(b"lpcm");
const FLAG_FLOAT: u32 = 1;
const FLAG_NON_INTERLEAVED: u32 = 1 << 5;

fn addr(selector: u32) -> PropertyAddress {
    PropertyAddress { selector, scope: SCOPE_GLOBAL, element: 0 }
}

unsafe fn sym<T: Copy>(handle: *mut c_void, name: &CStr) -> Option<T> {
    let p = libc::dlsym(handle, name.as_ptr());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy::<*mut c_void, T>(&p))
    }
}

fn tap_fns() -> Option<(CreateTapFn, DestroyTapFn)> {
    unsafe {
        let create = sym::<CreateTapFn>(libc::RTLD_DEFAULT, c"AudioHardwareCreateProcessTap")?;
        let destroy = sym::<DestroyTapFn>(libc::RTLD_DEFAULT, c"AudioHardwareDestroyProcessTap")?;
        Some((create, destroy))
    }
}

/// Поддерживает ли система захват через Core Audio tap (macOS 14.2+).
pub fn supported() -> bool {
    tap_fns().is_some() && AnyClass::get(c"CATapDescription").is_some()
}

// ---------- Разрешения ----------

/// Статус разрешения в духе TCC: `granted`, `denied`, `undetermined`, `unknown`.
pub type PermissionStatus = &'static str;

fn tcc_handle() -> *mut c_void {
    unsafe {
        libc::dlopen(
            c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(),
            libc::RTLD_NOW,
        )
    }
}

/// «Только запись системного звука» (kTCCServiceAudioCapture).
pub fn system_audio_status() -> PermissionStatus {
    // Результат — C `int` (32 бита): читать шире нельзя, старшие биты мусор.
    type Preflight = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
    let h = tcc_handle();
    if h.is_null() {
        return "unknown";
    }
    let Some(preflight) = (unsafe { sym::<Preflight>(h, c"TCCAccessPreflight") }) else {
        return "unknown";
    };
    let service = NSString::from_str("kTCCServiceAudioCapture");
    match unsafe { preflight(Retained::as_ptr(&service) as *const c_void, std::ptr::null()) } {
        0 => "granted",
        1 => "denied",
        2 => "undetermined",
        _ => "unknown",
    }
}

/// Показывает системный запрос «Только запись системного звука». Ответ
/// приходит асинхронно — фронтенд перечитывает статус.
pub fn request_system_audio() {
    type Request = unsafe extern "C" fn(*const c_void, *const c_void, *const c_void);
    let h = tcc_handle();
    let request = if h.is_null() { None } else { unsafe { sym::<Request>(h, c"TCCAccessRequest") } };
    match request {
        Some(request) => {
            let service = NSString::from_str("kTCCServiceAudioCapture");
            let done = block2::RcBlock::new(|_granted: Bool| {});
            unsafe {
                request(
                    Retained::as_ptr(&service) as *const c_void,
                    std::ptr::null(),
                    block2::RcBlock::as_ptr(&done) as *const c_void,
                )
            };
        }
        None => {
            // Без TCC-функций запрос появляется при первом запуске tap:
            // короткий пробный захват в никуда.
            std::thread::spawn(|| {
                let dir = std::env::temp_dir().join("memiro-audio-probe.wav");
                if let Ok(sink) = TrackSink::create(&dir, 48_000) {
                    if let Ok(tap) = SystemTap::start(Arc::new(Mutex::new(sink))) {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        drop(tap);
                    }
                }
                let _ = std::fs::remove_file(dir);
            });
        }
    }
}

/// Микрофон: `AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio`.
pub fn mic_status() -> PermissionStatus {
    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else { return "unknown" };
    let audio = NSString::from_str("soun"); // AVMediaTypeAudio
    let st: isize = unsafe { msg_send![cls, authorizationStatusForMediaType: &*audio] };
    match st {
        0 => "undetermined",
        1 | 2 => "denied",
        3 => "granted",
        _ => "unknown",
    }
}

/// Системный запрос доступа к микрофону.
pub fn request_mic() {
    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else { return };
    let audio = NSString::from_str("soun");
    let done = block2::RcBlock::new(|_granted: Bool| {});
    unsafe {
        let _: () = msg_send![cls, requestAccessForMediaType: &*audio, completionHandler: &*done];
    }
}

// ---------- Захват ----------

struct TapCtx {
    sink: Sink,
    channels: usize,
    non_interleaved: bool,
    /// Агрегатное устройство: его частота — частота приходящего звука.
    aggregate: u32,
    rate: std::sync::atomic::AtomicU32,
    calls: std::sync::atomic::AtomicU32,
}

/// Номинальная частота устройства (Гц), 0 — не удалось прочитать.
fn nominal_rate(device: u32) -> u32 {
    let a = addr(SEL_NOMINAL_RATE);
    let mut rate: f64 = 0.0;
    let mut size = std::mem::size_of::<f64>() as u32;
    let st = unsafe { AudioObjectGetPropertyData(device, &a, 0, std::ptr::null(), &mut size, &mut rate as *mut f64 as *mut c_void) };
    if st == 0 && rate.is_finite() && (4_000.0..=768_000.0).contains(&rate) {
        rate.round() as u32
    } else {
        0
    }
}

unsafe extern "C" fn io_proc(
    _device: u32,
    _now: *const c_void,
    input: *const AudioBufferList,
    _input_time: *const c_void,
    _output: *mut AudioBufferList,
    _output_time: *const c_void,
    client: *mut c_void,
) -> i32 {
    if input.is_null() || client.is_null() {
        return 0;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use std::sync::atomic::Ordering;
        let ctx = &*(client as *const TapCtx);
        let count = (*input).count as usize;
        let all = std::slice::from_raw_parts((*input).buffers.as_ptr(), count);
        // В агрегате сначала идут входы устройства вывода (у гарнитуры —
        // её микрофон), затем tap: берём только буферы tap — последние.
        let tap_bufs = if ctx.non_interleaved { ctx.channels.max(1) } else { 1 };
        let buffers = &all[count.saturating_sub(tap_bufs)..];
        let count = buffers.len();
        let Ok(mut sink) = ctx.sink.lock() else { return };
        // Частота устройства может смениться посреди записи (AirPods в
        // звонке переходят на 16/24 кГц) — раз в ~сотню блоков сверяемся,
        // иначе звук ускорится/замедлится и распознавание сломается.
        if ctx.calls.fetch_add(1, Ordering::Relaxed) % 128 == 0 {
            let r = nominal_rate(ctx.aggregate);
            if r != 0 && r != ctx.rate.load(Ordering::Relaxed) {
                ctx.rate.store(r, Ordering::Relaxed);
                sink.set_input_rate(r);
                eprintln!("system audio tap: rate → {r} Hz");
            }
        }
        if ctx.non_interleaved && count > 1 {
            // По буферу на канал — сводим в моно.
            let frames = buffers.iter().map(|b| b.size as usize / 4).min().unwrap_or(0);
            let mut mono = vec![0.0f32; frames];
            for b in buffers {
                if b.data.is_null() {
                    continue;
                }
                let ch = std::slice::from_raw_parts(b.data as *const f32, frames);
                for (m, x) in mono.iter_mut().zip(ch) {
                    *m += *x / count as f32;
                }
            }
            sink.push(&mono, 1);
        } else {
            for b in buffers {
                if b.data.is_null() || b.size == 0 {
                    continue;
                }
                let samples = std::slice::from_raw_parts(b.data as *const f32, b.size as usize / 4);
                let ch = if ctx.non_interleaved { 1 } else { (b.channels as usize).max(ctx.channels).max(1) };
                sink.push(samples, ch);
            }
        }
    }));
    0
}

/// Идущий захват системного звука. Останавливается при `drop`.
pub struct SystemTap {
    tap: u32,
    aggregate: u32,
    proc_id: Option<IoProc>,
    ctx: *mut TapCtx,
    destroy_tap: DestroyTapFn,
}

// Хендлы Core Audio — числа; контекст живёт до остановки.
unsafe impl Send for SystemTap {}

fn ns_obj<T: objc2::Message>(x: Retained<T>) -> Retained<NSObject>
where
    T: objc2::ClassType,
{
    // SAFETY: все используемые типы — наследники NSObject.
    unsafe { Retained::cast_unchecked(x) }
}

fn device_uid(device: u32) -> Option<Retained<NSString>> {
    let a = addr(SEL_DEVICE_UID);
    let mut uid: *mut NSString = std::ptr::null_mut();
    let mut size = std::mem::size_of::<*mut NSString>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(device, &a, 0, std::ptr::null(), &mut size, &mut uid as *mut _ as *mut c_void)
    };
    if st != 0 || uid.is_null() {
        return None;
    }
    // CFStringRef (+1) ↔ NSString — бесплатный мост.
    unsafe { Retained::from_raw(uid) }
}

impl SystemTap {
    /// Запускает tap: все процессы, кроме самого Memiro, в моно.
    pub fn start(sink: Sink) -> Result<Self, String> {
        let (create_tap, destroy_tap) = tap_fns().ok_or("Core Audio tap недоступен (нужна macOS 14.2+)")?;
        let cls = AnyClass::get(c"CATapDescription").ok_or("нет CATapDescription")?;

        // Свой процесс — в исключения, чтобы звуки Memiro не попадали в запись.
        let mut excluded: Vec<Retained<NSNumber>> = Vec::new();
        let pid: i32 = std::process::id() as i32;
        let a = addr(SEL_PID_TO_PROCESS);
        let mut me: u32 = 0;
        let mut size = 4u32;
        let st = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &a,
                4,
                &pid as *const i32 as *const c_void,
                &mut size,
                &mut me as *mut u32 as *mut c_void,
            )
        };
        if st == 0 && me != 0 {
            excluded.push(NSNumber::new_u32(me));
        }
        let excluded = NSArray::from_retained_slice(&excluded);

        let desc: Retained<AnyObject> = unsafe {
            let alloc: objc2::rc::Allocated<AnyObject> = msg_send![cls, alloc];
            let d: Option<Retained<AnyObject>> = msg_send![alloc, initMonoGlobalTapButExcludeProcesses: &*excluded];
            d.ok_or("CATapDescription init")?
        };
        // Приватный tap — не виден другим приложениям (если свойство есть).
        unsafe {
            let responds: bool = msg_send![&*desc, respondsToSelector: sel!(setPrivate:)];
            if responds {
                let _: () = msg_send![&*desc, setPrivate: Bool::YES];
            }
        }
        let tap_uuid: Retained<NSUUID> = unsafe { msg_send![&*desc, UUID] };
        let tap_uid = tap_uuid.UUIDString();

        let mut tap: u32 = 0;
        let st = unsafe { create_tap(Retained::as_ptr(&desc) as *mut AnyObject, &mut tap) };
        if st != 0 || tap == 0 {
            return Err(format!("AudioHardwareCreateProcessTap: {st}"));
        }

        // Формат tap (частота устройства вывода, float32).
        let a = addr(SEL_TAP_FORMAT);
        let mut fmt = StreamDescription::default();
        let mut size = std::mem::size_of::<StreamDescription>() as u32;
        let st = unsafe {
            AudioObjectGetPropertyData(tap, &a, 0, std::ptr::null(), &mut size, &mut fmt as *mut _ as *mut c_void)
        };
        if st != 0 || fmt.format_id != FORMAT_LPCM || fmt.format_flags & FLAG_FLOAT == 0 || fmt.bits_per_channel != 32 {
            unsafe { destroy_tap(tap) };
            return Err(format!("формат tap не поддержан (st {st}, flags {:#x})", fmt.format_flags));
        }

        // Агрегатное устройство: устройство вывода + наш tap.
        let a = addr(SEL_DEFAULT_SYSTEM_OUTPUT);
        let mut out_dev: u32 = 0;
        let mut size = 4u32;
        let st = unsafe {
            AudioObjectGetPropertyData(SYSTEM_OBJECT, &a, 0, std::ptr::null(), &mut size, &mut out_dev as *mut u32 as *mut c_void)
        };
        let out_uid = if st == 0 { device_uid(out_dev) } else { None };
        let Some(out_uid) = out_uid else {
            unsafe { destroy_tap(tap) };
            return Err("не найдено устройство вывода".into());
        };

        let key = NSString::from_str;
        let sub = NSDictionary::from_retained_objects(&[&*key("uid")], &[ns_obj(out_uid.clone())]);
        let tap_entry = NSDictionary::from_retained_objects(
            &[&*key("uid"), &*key("drift")],
            &[ns_obj(tap_uid), ns_obj(NSNumber::new_bool(true))],
        );
        let agg_uid = NSUUID::UUID().UUIDString();
        let dict = NSDictionary::from_retained_objects(
            &[
                &*key("name"),
                &*key("uid"),
                &*key("master"),
                &*key("private"),
                &*key("stacked"),
                &*key("tapautostart"),
                &*key("subdevices"),
                &*key("taps"),
            ],
            &[
                ns_obj(NSString::from_str("Memiro system audio")),
                ns_obj(agg_uid),
                ns_obj(out_uid),
                ns_obj(NSNumber::new_bool(true)),
                ns_obj(NSNumber::new_bool(false)),
                ns_obj(NSNumber::new_bool(true)),
                ns_obj(NSArray::from_retained_slice(&[sub])),
                ns_obj(NSArray::from_retained_slice(&[tap_entry])),
            ],
        );
        let mut aggregate: u32 = 0;
        let st = unsafe { AudioHardwareCreateAggregateDevice(Retained::as_ptr(&dict) as *const c_void, &mut aggregate) };
        if st != 0 || aggregate == 0 {
            unsafe { destroy_tap(tap) };
            return Err(format!("AudioHardwareCreateAggregateDevice: {st}"));
        }

        // Частота звука в IOProc — частота агрегата (ведущее устройство), а
        // не обязательно tap: при расхождении верим агрегату.
        let agg_rate = nominal_rate(aggregate);
        let rate = if agg_rate != 0 { agg_rate } else { fmt.sample_rate.round() as u32 };
        eprintln!(
            "system audio tap: tap {} Hz {} ch{}, aggregate {agg_rate} Hz",
            fmt.sample_rate,
            fmt.channels_per_frame,
            if fmt.format_flags & FLAG_NON_INTERLEAVED != 0 { " non-interleaved" } else { "" }
        );
        if let Ok(mut s) = sink.lock() {
            s.set_input_rate(rate);
        }
        let ctx = Box::into_raw(Box::new(TapCtx {
            sink,
            channels: fmt.channels_per_frame.max(1) as usize,
            non_interleaved: fmt.format_flags & FLAG_NON_INTERLEAVED != 0,
            aggregate,
            rate: std::sync::atomic::AtomicU32::new(rate),
            calls: std::sync::atomic::AtomicU32::new(1),
        }));
        let mut proc_id: Option<IoProc> = None;
        let st = unsafe { AudioDeviceCreateIOProcID(aggregate, io_proc, ctx as *mut c_void, &mut proc_id) };
        let mut this = SystemTap { tap, aggregate, proc_id: None, ctx, destroy_tap };
        if st != 0 || proc_id.is_none() {
            return Err(format!("AudioDeviceCreateIOProcID: {st}"));
        }
        this.proc_id = proc_id;
        let st = unsafe { AudioDeviceStart(aggregate, proc_id) };
        if st != 0 {
            return Err(format!("AudioDeviceStart: {st}"));
        }
        Ok(this)
    }
}

impl Drop for SystemTap {
    fn drop(&mut self) {
        unsafe {
            if self.proc_id.is_some() {
                AudioDeviceStop(self.aggregate, self.proc_id);
                AudioDeviceDestroyIOProcID(self.aggregate, self.proc_id);
            }
            AudioHardwareDestroyAggregateDevice(self.aggregate);
            (self.destroy_tap)(self.tap);
            if !self.ctx.is_null() {
                drop(Box::from_raw(self.ctx));
            }
        }
    }
}
