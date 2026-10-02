use serde::{Deserialize, Serialize};

/// Идентификатор говорящего в ленте. Для записанных встреч это `"me"`
/// (микрофон) и `"them"` (системный звук); для импортированных записей —
/// `"spk0"`, `"spk1"`, … после диаризации (или один `"spk0"` без неё).
/// Хранится строкой: модель расширяема на N говорящих, а старые
/// `transcript.json` (где `speaker` уже сериализовался в `"me"`/`"them"`)
/// читаются без изменений.
pub const ME: &str = "me";
pub const THEM: &str = "them";

/// Человеко-читаемая подпись говорящего по его id.
pub fn speaker_label(id: &str) -> String {
    match id {
        ME => "Я".to_string(),
        THEM => "Собеседник".to_string(),
        other => match other.strip_prefix("spk").and_then(|n| n.parse::<usize>().ok()) {
            Some(n) => format!("Спикер {}", n + 1),
            None => other.to_string(),
        },
    }
}

/// Сырой сегмент от транскрайбера (одна дорожка).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start_secs: f64,
    pub end_secs: f64,
    pub text: String,
}

/// Сегмент итоговой ленты, с говорящим (id, см. [`speaker_label`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub speaker: String,
    pub start_secs: f64,
    pub end_secs: f64,
    pub text: String,
    /// Чья реплика: `None` — распознана; `"user"` — добавлена пользователем,
    /// `"edited"` — текст исправлен пользователем. Такие реплики не трогают
    /// ни фоновое уточнение, ни повторная расшифровка.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl TranscriptSegment {
    /// Реплика пользователя (добавленная или исправленная).
    pub fn is_user(&self) -> bool {
        self.origin.is_some()
    }
}

/// Повторная расшифровка: реплики пользователя из прежней расшифровки
/// сохраняются, а распознанные заново, которые по времени в основном
/// приходятся на них (≥ половины длительности), отбрасываются — пользователь
/// уже записал это место.
pub fn keep_user_segments(old: &Transcript, fresh: Transcript) -> Transcript {
    // Пустой пузырь, который так и не заполнили, — не реплика.
    let user: Vec<&TranscriptSegment> =
        old.segments.iter().filter(|s| s.is_user() && !s.text.trim().is_empty()).collect();
    if user.is_empty() {
        return fresh;
    }
    let mut segments: Vec<TranscriptSegment> =
        fresh.segments.into_iter().filter(|f| !covered_by_user(f.start_secs, f.end_secs, &user)).collect();
    segments.extend(user.into_iter().cloned());
    segments.sort_by(|a, b| a.start_secs.partial_cmp(&b.start_secs).unwrap_or(std::cmp::Ordering::Equal));
    Transcript { segments }
}

/// Отрезок `a..b` в основном (≥ 50 %) приходится на реплики пользователя.
pub fn covered_by_user(a: f64, b: f64, user: &[&TranscriptSegment]) -> bool {
    let len = (b - a).max(0.05);
    let overlap: f64 = user.iter().map(|u| (b.min(u.end_secs) - a.max(u.start_secs)).max(0.0)).sum();
    overlap >= 0.5 * len
}

/// Сегмент диаризации: интервал и сырой номер говорящего (id кластера).
/// Производится [`crate::diarize::Diarizer`], потребляется [`assign_speakers`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiarSegment {
    pub start_secs: f64,
    pub end_secs: f64,
    pub speaker: u32,
}

/// Полная расшифровка встречи.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
}

/// Объединяет сегменты микрофона («Я») и системного звука («Собеседник»)
/// в единую ленту, отсортированную по времени начала. Пустые тексты
/// пропускаются. Сортировка стабильна: при равном времени «Я» идёт раньше.
pub fn merge_tracks(mic: Vec<Segment>, system: Vec<Segment>) -> Transcript {
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    for s in mic {
        if s.text.trim().is_empty() {
            continue;
        }
        segments.push(TranscriptSegment { origin: None,
            speaker: ME.to_string(),
            start_secs: s.start_secs,
            end_secs: s.end_secs,
            text: s.text,
        });
    }
    for s in system {
        if s.text.trim().is_empty() {
            continue;
        }
        segments.push(TranscriptSegment { origin: None,
            speaker: THEM.to_string(),
            start_secs: s.start_secs,
            end_secs: s.end_secs,
            text: s.text,
        });
    }
    segments.sort_by(|a, b| {
        a.start_secs
            .partial_cmp(&b.start_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Transcript { segments }
}

/// Строит ленту из ОДНОЙ дорожки: всем сегментам присваивается один говорящий
/// `speaker`. Пустые тексты пропускаются. Для импортированных записей без
/// диаризации (один голос); диаризация на несколько говорящих появится в M3.
/// Живая встреча, записанная микрофоном: в дорожке звонка речи почти нет
/// (меньше 10 % слов микрофона), а в микрофоне она есть (≥ 20 слов). Тогда
/// голоса делятся по дорожке микрофона, а не по дорожке звонка.
pub fn is_in_person(mic: &[Segment], system: &[Segment]) -> bool {
    let words = |v: &[Segment]| v.iter().map(|s| s.text.split_whitespace().count()).sum::<usize>();
    let (m, sys) = (words(mic), words(system));
    m >= 20 && sys * 10 < m
}

pub fn single_speaker(segments: Vec<Segment>, speaker: &str) -> Transcript {
    let segments = segments
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| TranscriptSegment { origin: None,
            speaker: speaker.to_string(),
            start_secs: s.start_secs,
            end_secs: s.end_secs,
            text: s.text,
        })
        .collect();
    Transcript { segments }
}

/// Склеивает текст whisper с разметкой говорящих от диаризации: каждому
/// текстовому сегменту назначается говорящий с максимальным перекрытием по
/// времени (при отсутствии перекрытия — ближайший по середине). Сырые номера
/// говорящих перенумеровываются в `spk0`, `spk1`, … по порядку появления.
/// Пустой `diar` ⇒ один говорящий `spk0`.
pub fn assign_speakers(whisper: Vec<Segment>, diar: Vec<DiarSegment>) -> Transcript {
    use std::collections::HashMap;
    let mut remap: HashMap<u32, usize> = HashMap::new();
    let mut next = 0usize;
    let mut segments = Vec::new();
    for s in whisper {
        if s.text.trim().is_empty() {
            continue;
        }
        let speaker = match best_speaker(&diar, s.start_secs, s.end_secs) {
            Some(raw) => {
                let idx = *remap.entry(raw).or_insert_with(|| {
                    let i = next;
                    next += 1;
                    i
                });
                format!("spk{idx}")
            }
            None => "spk0".to_string(),
        };
        segments.push(TranscriptSegment { origin: None,
            speaker,
            start_secs: s.start_secs,
            end_secs: s.end_secs,
            text: s.text,
        });
    }
    Transcript { segments }
}

/// Переразмечает говорящих у реплик, для которых `pick` истинно, по разметке
/// диаризации (`spk0`, `spk1`… по порядку появления; прочие реплики не
/// трогаются). Если `single` задан и голос вышел один — всем выбранным
/// репликам ставится `single` (напр. «Собеседник» для записанной встречи).
pub fn relabel_speakers(
    transcript: &Transcript,
    diar: &[DiarSegment],
    pick: impl Fn(&TranscriptSegment) -> bool,
    single: Option<&str>,
) -> Transcript {
    use std::collections::HashMap;
    let mut remap: HashMap<u32, usize> = HashMap::new();
    let mut segments = transcript.segments.clone();
    let mut picked = Vec::new();
    for (i, s) in segments.iter_mut().enumerate() {
        if !pick(s) {
            continue;
        }
        picked.push(i);
        let idx = match best_speaker(diar, s.start_secs, s.end_secs) {
            Some(raw) => {
                let next = remap.len();
                *remap.entry(raw).or_insert(next)
            }
            None => 0,
        };
        s.speaker = format!("spk{idx}");
    }
    if let Some(name) = single {
        if remap.len() <= 1 {
            for i in picked {
                segments[i].speaker = name.to_string();
            }
        }
    }
    Transcript { segments }
}

/// Если во всей ленте один голос `spkN` — заменяет его на `name`.
pub fn collapse_single_speaker(t: Transcript, name: &str) -> Transcript {
    let mut ids: Vec<&str> = t.segments.iter().map(|s| s.speaker.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != 1 {
        return t;
    }
    Transcript {
        segments: t
            .segments
            .into_iter()
            .map(|mut s| {
                s.speaker = name.to_string();
                s
            })
            .collect(),
    }
}

/// Объединяет две готовые ленты в одну, отсортированную по времени начала
/// (стабильно). Для записей с несколькими собеседниками: дорожка «Я» (микрофон)
/// + диаризованная дорожка собеседников (системный звук).
pub fn merge_transcripts(a: Transcript, b: Transcript) -> Transcript {
    let mut segments = a.segments;
    segments.extend(b.segments);
    segments.sort_by(|x, y| {
        x.start_secs
            .partial_cmp(&y.start_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Transcript { segments }
}

/// Сырой номер говорящего с максимальным перекрытием интервала [start,end].
/// Если ни один не перекрывается — ближайший по середине. `None` при пустом diar.
fn best_speaker(diar: &[DiarSegment], start: f64, end: f64) -> Option<u32> {
    let mut best: Option<(f64, u32)> = None;
    for d in diar {
        let overlap = (end.min(d.end_secs) - start.max(d.start_secs)).max(0.0);
        if overlap > 0.0 && best.map(|(o, _)| overlap > o).unwrap_or(true) {
            best = Some((overlap, d.speaker));
        }
    }
    if let Some((_, sp)) = best {
        return Some(sp);
    }
    // Нет перекрытия — ближайший по середине сегмента.
    let mid = (start + end) / 2.0;
    diar.iter()
        .min_by(|a, b| {
            dist_to(mid, a)
                .partial_cmp(&dist_to(mid, b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|d| d.speaker)
}

fn dist_to(mid: f64, d: &DiarSegment) -> f64 {
    if mid < d.start_secs {
        d.start_secs - mid
    } else if mid > d.end_secs {
        mid - d.end_secs
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_in_person_meetings() {
        let seg = |n: usize| Segment { start_secs: 0.0, end_secs: 1.0, text: vec!["слово"; n].join(" ") };
        // Живая встреча: всё в микрофоне.
        assert!(is_in_person(&[seg(30), seg(40)], &[]));
        assert!(is_in_person(&[seg(100)], &[seg(3)]));
        // Звонок: собеседники в дорожке звонка.
        assert!(!is_in_person(&[seg(100)], &[seg(60)]));
        // Слишком мало речи, чтобы судить.
        assert!(!is_in_person(&[seg(10)], &[]));
    }

    fn seg(start: f64, text: &str) -> Segment {
        Segment { start_secs: start, end_secs: start + 1.0, text: text.into() }
    }

    #[test]
    fn merges_and_orders_by_start_time() {
        let mic = vec![seg(0.0, "привет"), seg(4.0, "как дела")];
        let system = vec![seg(2.0, "здравствуй")];
        let t = merge_tracks(mic, system);
        let order: Vec<(&str, &str)> =
            t.segments.iter().map(|s| (s.speaker.as_str(), s.text.as_str())).collect();
        assert_eq!(
            order,
            vec![
                ("me", "привет"),
                ("them", "здравствуй"),
                ("me", "как дела"),
            ]
        );
    }

    #[test]
    fn skips_empty_text() {
        let mic = vec![seg(0.0, "  "), seg(1.0, "ok")];
        let t = merge_tracks(mic, vec![]);
        assert_eq!(t.segments.len(), 1);
        assert_eq!(t.segments[0].text, "ok");
    }

    #[test]
    fn tie_breaks_me_before_them() {
        let mic = vec![seg(1.0, "я")];
        let system = vec![seg(1.0, "он")];
        let t = merge_tracks(mic, system);
        assert_eq!(t.segments[0].speaker, "me");
        assert_eq!(t.segments[1].speaker, "them");
    }

    #[test]
    fn single_speaker_assigns_one_and_skips_empty() {
        let segs = vec![seg(0.0, "первая"), seg(1.0, "  "), seg(2.0, "вторая")];
        let t = single_speaker(segs, "spk0");
        assert_eq!(t.segments.len(), 2);
        assert!(t.segments.iter().all(|s| s.speaker == "spk0"));
    }

    #[test]
    fn speaker_label_maps_known_and_spk() {
        assert_eq!(speaker_label("me"), "Я");
        assert_eq!(speaker_label("them"), "Собеседник");
        assert_eq!(speaker_label("spk0"), "Спикер 1");
        assert_eq!(speaker_label("spk2"), "Спикер 3");
        assert_eq!(speaker_label("custom"), "custom");
    }

    #[test]
    fn assign_speakers_by_max_overlap_and_renumbers() {
        // seg(start) занимает [start, start+1].
        let whisper = vec![seg(0.0, "привет"), seg(5.0, "как дела"), seg(0.5, "ещё я")];
        // Сырые номера кластеров 7 и 3 — должны перенумероваться в spk0/spk1.
        let diar = vec![
            DiarSegment { start_secs: 0.0, end_secs: 2.0, speaker: 7 },
            DiarSegment { start_secs: 4.0, end_secs: 8.0, speaker: 3 },
        ];
        let t = assign_speakers(whisper, diar);
        assert_eq!(t.segments[0].speaker, "spk0"); // перекрытие с кластером 7
        assert_eq!(t.segments[1].speaker, "spk1"); // перекрытие с кластером 3
        assert_eq!(t.segments[2].speaker, "spk0"); // снова кластер 7
    }

    #[test]
    fn merge_transcripts_orders_by_start() {
        let me = single_speaker(vec![seg(0.0, "я раз"), seg(4.0, "я два")], "me");
        let them = single_speaker(vec![seg(2.0, "он")], "spk0");
        let t = merge_transcripts(me, them);
        let order: Vec<(&str, &str)> =
            t.segments.iter().map(|s| (s.speaker.as_str(), s.text.as_str())).collect();
        assert_eq!(
            order,
            vec![("me", "я раз"), ("spk0", "он"), ("me", "я два")]
        );
    }

    #[test]
    fn assign_speakers_empty_diar_is_single_speaker() {
        let t = assign_speakers(vec![seg(0.0, "a"), seg(2.0, "b")], vec![]);
        assert_eq!(t.segments.len(), 2);
        assert!(t.segments.iter().all(|s| s.speaker == "spk0"));
    }

    #[test]
    fn assign_speakers_no_overlap_picks_nearest() {
        // Текст в [10,11], диаризация рядом — берём ближайший кластер.
        let whisper = vec![seg(10.0, "поздняя реплика")];
        let diar = vec![
            DiarSegment { start_secs: 0.0, end_secs: 5.0, speaker: 1 },
            DiarSegment { start_secs: 8.0, end_secs: 9.0, speaker: 2 },
        ];
        let t = assign_speakers(whisper, diar);
        // Ближайший по середине (10.5) — кластер 2 (первый назначенный → spk0).
        assert_eq!(t.segments[0].speaker, "spk0");
    }

    #[test]
    fn rerun_keeps_user_phrases() {
        let user = |a: f64, b: f64, t: &str, o: &str| TranscriptSegment {
            speaker: "spk1".into(),
            start_secs: a,
            end_secs: b,
            text: t.into(),
            origin: Some(o.into()),
        };
        let old = Transcript {
            segments: vec![
                TranscriptSegment { speaker: ME.into(), start_secs: 0.0, end_secs: 2.0, text: "старое".into(), origin: None },
                user(3.0, 5.0, "добавил я", "user"),
                user(6.0, 8.0, "исправил я", "edited"),
                user(8.5, 9.0, " ", "user"),
            ],
        };
        let fresh = merge_tracks(vec![seg(0.0, "новое"), seg(3.2, "мусор"), seg(9.0, "дальше")], vec![]);
        let fresh = Transcript {
            segments: fresh
                .segments
                .into_iter()
                .map(|mut s| {
                    if s.text == "мусор" {
                        s.end_secs = 4.5;
                    }
                    s
                })
                .chain([TranscriptSegment { speaker: ME.into(), start_secs: 6.1, end_secs: 7.9, text: "ошибка".into(), origin: None }])
                .collect(),
        };
        let out = keep_user_segments(&old, fresh);
        let texts: Vec<&str> = out.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["новое", "добавил я", "исправил я", "дальше"]);
    }

    #[test]
    fn relabel_keeps_me_and_collapses_single_voice() {
        let t = merge_tracks(vec![seg(0.0, "я")], vec![seg(2.0, "он"), seg(5.0, "снова он")]);
        let one = vec![DiarSegment { start_secs: 0.0, end_secs: 9.0, speaker: 4 }];
        let r = relabel_speakers(&t, &one, |s| s.speaker != ME, Some(THEM));
        let sp: Vec<&str> = r.segments.iter().map(|s| s.speaker.as_str()).collect();
        assert_eq!(sp, vec!["me", "them", "them"]);

        let two = vec![
            DiarSegment { start_secs: 0.0, end_secs: 4.0, speaker: 9 },
            DiarSegment { start_secs: 4.0, end_secs: 9.0, speaker: 2 },
        ];
        let r = relabel_speakers(&t, &two, |s| s.speaker != ME, Some(THEM));
        let sp: Vec<&str> = r.segments.iter().map(|s| s.speaker.as_str()).collect();
        assert_eq!(sp, vec!["me", "spk0", "spk1"]);
        // Текст не трогается.
        assert_eq!(r.segments[2].text, "снова он");
    }

    #[test]
    fn collapse_single_only_when_one_voice() {
        let t = single_speaker(vec![seg(0.0, "a"), seg(1.0, "b")], "spk0");
        assert!(collapse_single_speaker(t, THEM).segments.iter().all(|s| s.speaker == THEM));
        let t = assign_speakers(
            vec![seg(0.0, "a"), seg(5.0, "b")],
            vec![
                DiarSegment { start_secs: 0.0, end_secs: 2.0, speaker: 0 },
                DiarSegment { start_secs: 4.0, end_secs: 7.0, speaker: 1 },
            ],
        );
        assert_eq!(collapse_single_speaker(t.clone(), THEM), t);
    }

    #[test]
    fn transcript_round_trips_json() {
        let t = merge_tracks(vec![seg(0.0, "hi")], vec![]);
        let json = serde_json::to_string(&t).unwrap();
        let back: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
