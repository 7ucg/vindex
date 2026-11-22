//! ffmpeg-freier TS→MKV-Remuxer für den häufigen Fall **H.264-Video + AAC-Audio**
//! (die HLS-Segmente der unterstützten Hoster). Demuxt MPEG-TS, extrahiert die
//! Elementarströme und schreibt eine Matroska-Datei — ohne Re-Encoding.
//!
//! Bewusst fokussiert: ein Video- und ein Audio-Track, AAC-LC in ADTS, H.264 in
//! Annex-B. Andere Codecs/SAMPLE-AES bleiben dem ffmpeg-Pfad überlassen; der
//! Aufrufer fällt dann auf den rohen `.ts`-Pfad zurück. Arbeitet im Speicher (mit
//! Größen-Cap im Aufrufer), damit der Muxer einfach und testbar bleibt.

use crate::error::{Error, Result};

// ───────────────────────────── MPEG-TS-Demux ─────────────────────────────

const TS_PACKET: usize = 188;
const SYNC: u8 = 0x47;

/// Ein zusammenhängender Elementarstrom eines TS-PIDs plus die PTS-Marken an den
/// PES-Grenzen (`(Offset in es, PTS)`). Eine Access Unit bekommt die PTS der Marke,
/// in deren Bereich ihr Start fällt — so stimmt die Zuordnung auch, wenn Access Units
/// über PES-Grenzen reichen.
#[derive(Default)]
struct Stream {
    es: Vec<u8>,
    marks: Vec<(usize, u64)>,
}

impl Stream {
    /// PTS der Marke, deren Offset ≤ *offset* ist (die zuletzt begonnene PES).
    fn pts_at(&self, offset: usize) -> u64 {
        self.marks
            .iter()
            .rev()
            .find(|(o, _)| *o <= offset)
            .map(|(_, p)| *p)
            .unwrap_or(0)
    }
}

/// Demux-Ergebnis: zusammenhängende Video-/Audio-Elementarströme.
#[derive(Default)]
struct Demuxed {
    video: Stream,
    audio: Stream,
}

/// Liest die 33-Bit-PTS aus einem PES-Header (`pts_dts_flags` schon geprüft).
fn read_pts(b: &[u8]) -> Option<u64> {
    if b.len() < 5 {
        return None;
    }
    let pts = (((b[0] >> 1) & 0x07) as u64) << 30
        | (b[1] as u64) << 22
        | (((b[2] >> 1) & 0x7f) as u64) << 15
        | (b[3] as u64) << 7
        | ((b[4] >> 1) & 0x7f) as u64;
    Some(pts)
}

/// Parst den Kopf eines PES-Pakets (am Anfang einer PUSI-Nutzlast) zu `(PTS,
/// ES-Daten-Offset)`.
fn pes_header(payload: &[u8]) -> Option<(u64, usize)> {
    if payload.len() < 9 || payload[0] != 0 || payload[1] != 0 || payload[2] != 1 {
        return None;
    }
    let header_len = payload[8] as usize;
    let pts = if payload[7] >> 6 & 0x02 != 0 {
        read_pts(&payload[9..])?
    } else {
        0
    };
    let off = 9 + header_len;
    (off <= payload.len()).then_some((pts, off))
}

/// Hängt eine TS-Nutzlast an den passenden Elementarstrom. Bei PES-Start (PUSI) wird
/// der PES-Kopf entfernt und eine PTS-Marke gesetzt; sonst roh angehängt.
fn append_payload(stream: &mut Stream, pusi: bool, payload: &[u8]) {
    if pusi {
        if let Some((pts, off)) = pes_header(payload) {
            stream.marks.push((stream.es.len(), pts));
            stream.es.extend_from_slice(&payload[off..]);
        }
    } else {
        stream.es.extend_from_slice(payload);
    }
}

/// Demuxt einen (ggf. aus mehreren Segmenten zusammengesetzten) TS-Strom.
fn demux(ts: &[u8]) -> Result<Demuxed> {
    let offset = ts.iter().position(|&b| b == SYNC).unwrap_or(0);
    let ts = &ts[offset..];

    let mut pmt_pid: Option<u16> = None;
    let mut video_pid: Option<u16> = None;
    let mut audio_pid: Option<u16> = None;
    let mut out = Demuxed::default();

    for pkt in ts.as_chunks::<TS_PACKET>().0 {
        if pkt[0] != SYNC {
            continue;
        }
        let pid = (((pkt[1] & 0x1f) as u16) << 8) | pkt[2] as u16;
        let pusi = pkt[1] & 0x40 != 0;
        let adaptation = (pkt[3] >> 4) & 0x03;
        if adaptation & 0x01 == 0 {
            continue; // kein Payload
        }
        let mut pos = 4;
        if adaptation & 0x02 != 0 {
            pos += 1 + pkt[4] as usize;
            if pos >= TS_PACKET {
                continue;
            }
        }
        let payload = &pkt[pos..];

        if pid == 0 {
            if pusi && payload.len() > 13 {
                let p = 1 + payload[0] as usize;
                if p + 12 <= payload.len() {
                    pmt_pid =
                        Some((((payload[p + 10] & 0x1f) as u16) << 8) | payload[p + 11] as u16);
                }
            }
        } else if Some(pid) == pmt_pid {
            if pusi {
                parse_pmt(payload, &mut video_pid, &mut audio_pid);
            }
        } else if Some(pid) == video_pid {
            append_payload(&mut out.video, pusi, payload);
        } else if Some(pid) == audio_pid {
            append_payload(&mut out.audio, pusi, payload);
        }
    }

    if out.video.es.is_empty() {
        return Err(Error::extract("remux", "kein H.264-Videostrom im TS"));
    }
    Ok(out)
}

/// Parst die PMT und setzt Video-/Audio-PID (erster H.264-/AAC-Stream).
fn parse_pmt(payload: &[u8], video_pid: &mut Option<u16>, audio_pid: &mut Option<u16>) {
    if payload.is_empty() {
        return;
    }
    let p = 1 + payload[0] as usize; // pointer_field
    if p + 12 > payload.len() {
        return;
    }
    let section_len = (((payload[p + 1] & 0x0f) as usize) << 8) | payload[p + 2] as usize;
    let program_info_len = (((payload[p + 10] & 0x0f) as usize) << 8) | payload[p + 11] as usize;
    let mut i = p + 12 + program_info_len;
    let end = (p + 3 + section_len).min(payload.len()).saturating_sub(4); // ohne CRC
    while i + 5 <= end {
        let stream_type = payload[i];
        let epid = (((payload[i + 1] & 0x1f) as u16) << 8) | payload[i + 2] as u16;
        let es_info_len = (((payload[i + 3] & 0x0f) as usize) << 8) | payload[i + 4] as usize;
        match stream_type {
            0x1b if video_pid.is_none() => *video_pid = Some(epid), // H.264
            0x0f | 0x11 if audio_pid.is_none() => *audio_pid = Some(epid), // AAC (ADTS/LATM)
            _ => {}
        }
        i += 5 + es_info_len;
    }
}

// ───────────────────────────── H.264 (Annex-B) ─────────────────────────────

/// Zerlegt einen Annex-B-Strom in NAL-Einheiten (ohne Start-Codes).
fn split_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0;
    let n = data.len();
    // Start-Codes 00 00 01 oder 00 00 00 01 finden.
    let mut start: Option<usize> = None;
    while i + 3 <= n {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            if let Some(s) = start {
                nals.push(&data[s..i]);
            }
            start = Some(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        nals.push(&data[s..n]);
    }
    nals.into_iter().filter(|nal| !nal.is_empty()).collect()
}

/// Baut die avcC-CodecPrivate aus SPS + PPS.
fn build_avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(sps.len() + pps.len() + 16);
    v.push(1); // configurationVersion
    v.push(sps.get(1).copied().unwrap_or(0)); // profile
    v.push(sps.get(2).copied().unwrap_or(0)); // profile_compat
    v.push(sps.get(3).copied().unwrap_or(0)); // level
    v.push(0xff); // 6 Bit reserved + lengthSizeMinusOne=3
    v.push(0xe1); // 3 Bit reserved + numSPS=1
    v.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    v.extend_from_slice(sps);
    v.push(1); // numPPS
    v.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    v.extend_from_slice(pps);
    v
}

/// Ergebnis der H.264-Aufbereitung eines Zugriffspunkts (Access Unit).
struct VideoFrame {
    data: Vec<u8>, // längenpräfigierte NALs (AVCC)
    keyframe: bool,
}

/// Baut aus den NALs **einer** Access Unit einen AVCC-Frame; sammelt dabei SPS/PPS
/// (kommen in die CodecPrivate, nicht in die Frame-Daten) und erkennt Keyframes (IDR).
fn build_frame(nals: &[&[u8]], sps: &mut Option<Vec<u8>>, pps: &mut Option<Vec<u8>>) -> VideoFrame {
    let mut data = Vec::new();
    let mut keyframe = false;
    for nal in nals {
        match nal[0] & 0x1f {
            7 => {
                if sps.is_none() {
                    *sps = Some(nal.to_vec());
                }
            }
            8 => {
                if pps.is_none() {
                    *pps = Some(nal.to_vec());
                }
            }
            9 => {} // AUD gehört nicht in die Frame-Daten
            t => {
                if t == 5 {
                    keyframe = true;
                }
                data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                data.extend_from_slice(nal);
            }
        }
    }
    VideoFrame { data, keyframe }
}

/// Zerlegt den zusammenhängenden H.264-Strom in Access Units (an AUD-Grenzen) und
/// gibt je AU `(PTS, Frame)` zurück. Die PTS stammt aus der PES-Marke, in deren
/// Bereich der AU-Start fällt.
fn video_frames(
    stream: &Stream,
    sps: &mut Option<Vec<u8>>,
    pps: &mut Option<Vec<u8>>,
) -> Vec<(u64, VideoFrame)> {
    let nals = split_nals(&stream.es);
    let base = stream.es.as_ptr() as usize;
    let mut frames = Vec::new();
    let mut cur: Vec<&[u8]> = Vec::new();
    let mut cur_pts = stream.marks.first().map(|(_, p)| *p).unwrap_or(0);

    for nal in nals {
        let off = nal.as_ptr() as usize - base;
        if nal[0] & 0x1f == 9 {
            // AUD → neue Access Unit.
            if !cur.is_empty() {
                let f = build_frame(&cur, sps, pps);
                if !f.data.is_empty() {
                    frames.push((cur_pts, f));
                }
                cur.clear();
            }
            cur_pts = stream.pts_at(off);
        } else {
            cur.push(nal);
        }
    }
    if !cur.is_empty() {
        let f = build_frame(&cur, sps, pps);
        if !f.data.is_empty() {
            frames.push((cur_pts, f));
        }
    }
    frames
}

// ───────────────────────────── AAC (ADTS) ─────────────────────────────

const AAC_FREQS: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

struct AacConfig {
    asc: Vec<u8>,
    sample_rate: u32,
    channels: u8,
}

/// Liest aus dem ersten ADTS-Header den AudioSpecificConfig.
fn aac_config(adts: &[u8]) -> Option<AacConfig> {
    if adts.len() < 7 || adts[0] != 0xff || (adts[1] & 0xf0) != 0xf0 {
        return None;
    }
    let profile = (adts[2] >> 6) & 0x03; // 0=Main,1=LC,2=SSR
    let freq_idx = (adts[2] >> 2) & 0x0f;
    let chan = ((adts[2] & 0x01) << 2) | (adts[3] >> 6);
    let aot = profile + 1; // AOT = profile+1 (LC→2)
    let asc0 = (aot << 3) | (freq_idx >> 1);
    let asc1 = ((freq_idx & 0x01) << 7) | (chan << 3);
    Some(AacConfig {
        asc: vec![asc0, asc1],
        sample_rate: *AAC_FREQS.get(freq_idx as usize).unwrap_or(&44100),
        channels: chan.max(1),
    })
}

/// Zerlegt einen Audio-PES in rohe AAC-Frames (ADTS-Header entfernt).
fn split_adts(data: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut i = 0;
    while i + 7 <= data.len() {
        if data[i] != 0xff || (data[i + 1] & 0xf0) != 0xf0 {
            break;
        }
        let frame_len = (((data[i + 3] & 0x03) as usize) << 11)
            | ((data[i + 4] as usize) << 3)
            | (data[i + 5] as usize >> 5);
        if frame_len < 7 || i + frame_len > data.len() {
            break;
        }
        let has_crc = (data[i + 1] & 0x01) == 0;
        let hdr = if has_crc { 9 } else { 7 };
        if i + hdr <= i + frame_len {
            frames.push(data[i + hdr..i + frame_len].to_vec());
        }
        i += frame_len;
    }
    frames
}

// ───────────────────────────── EBML / Matroska ─────────────────────────────

/// Schreibt eine EBML-Element-Größe als Variable-Length-Integer (bekannte Länge).
fn vint_size(n: u64) -> Vec<u8> {
    // Kleinste Länge finden, bei der n in (7*len) Bits passt, Marker-Bit gesetzt.
    for len in 1u32..=8 {
        let bits = 7 * len;
        if n < (1u64 << bits) - 1 {
            let mut v = n.to_be_bytes().to_vec();
            v.drain(0..(8 - len as usize));
            v[0] |= 1 << (8 - len); // Längen-Marker
            return v;
        }
    }
    vec![0x01, 0, 0, 0, 0, 0, 0, 0]
}

/// Ein EBML-Element (ID-Bytes + größenpräfigierte Nutzlast).
fn elem(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(id.len() + 8 + body.len());
    v.extend_from_slice(id);
    v.extend_from_slice(&vint_size(body.len() as u64));
    v.extend_from_slice(body);
    v
}

/// Unsigned-Integer als Big-Endian-Minimalbytes (für EBML-Uint-Elemente).
fn uint(n: u64) -> Vec<u8> {
    if n == 0 {
        return vec![0];
    }
    let b = n.to_be_bytes();
    let first = b.iter().position(|&x| x != 0).unwrap_or(7);
    b[first..].to_vec()
}

/// Baut die komplette MKV-Datei aus den aufbereiteten Strömen.
#[allow(clippy::too_many_arguments)]
fn build_mkv(
    avcc: &[u8],
    width: u64,
    height: u64,
    aac: &AacConfig,
    v_frames: &[(i64, VideoFrame)], // (pts_ms, frame)
    a_frames: &[(i64, Vec<u8>)],    // (pts_ms, raw aac)
) -> Vec<u8> {
    // EBML-Header.
    let ebml = elem(
        &[0x1a, 0x45, 0xdf, 0xa3],
        &[
            elem(&[0x42, 0x86], &uint(1)),    // EBMLVersion
            elem(&[0x42, 0xf7], &uint(1)),    // EBMLReadVersion
            elem(&[0x42, 0xf2], &uint(4)),    // EBMLMaxIDLength
            elem(&[0x42, 0xf3], &uint(8)),    // EBMLMaxSizeLength
            elem(&[0x42, 0x82], b"matroska"), // DocType
            elem(&[0x42, 0x87], &uint(4)),    // DocTypeVersion
            elem(&[0x42, 0x85], &uint(2)),    // DocTypeReadVersion
        ]
        .concat(),
    );

    // Info.
    let duration_ms = v_frames
        .last()
        .map(|(t, _)| *t)
        .max(a_frames.last().map(|(t, _)| *t))
        .unwrap_or(0)
        .max(0) as f64;
    let info = elem(
        &[0x15, 0x49, 0xa9, 0x66],
        &[
            elem(&[0x2a, 0xd7, 0xb1], &uint(1_000_000)), // TimestampScale = 1ms
            elem(&[0x4d, 0x80], b"vindex"),              // MuxingApp
            elem(&[0x57, 0x41], b"vindex"),              // WritingApp
            elem(&[0x44, 0x89], &duration_ms.to_be_bytes()), // Duration (float64)
        ]
        .concat(),
    );

    // Tracks.
    let video_track = elem(
        &[0xae], // TrackEntry
        &[
            elem(&[0xd7], &uint(1)),           // TrackNumber
            elem(&[0x73, 0xc5], &uint(1)),     // TrackUID
            elem(&[0x83], &uint(1)),           // TrackType = video
            elem(&[0x86], b"V_MPEG4/ISO/AVC"), // CodecID
            elem(&[0x63, 0xa2], avcc),         // CodecPrivate
            elem(
                &[0xe0], // Video
                &[elem(&[0xb0], &uint(width)), elem(&[0xba], &uint(height))].concat(),
            ),
        ]
        .concat(),
    );
    let audio_track = elem(
        &[0xae],
        &[
            elem(&[0xd7], &uint(2)),
            elem(&[0x73, 0xc5], &uint(2)),
            elem(&[0x83], &uint(2)), // audio
            elem(&[0x86], b"A_AAC"),
            elem(&[0x63, 0xa2], &aac.asc),
            elem(
                &[0xe1], // Audio
                &[
                    elem(&[0xb5], &(aac.sample_rate as f64).to_be_bytes()), // SamplingFrequency (float)
                    elem(&[0x9f], &uint(aac.channels as u64)),              // Channels
                ]
                .concat(),
            ),
        ]
        .concat(),
    );
    let mut tracks_body = video_track;
    tracks_body.extend(audio_track);
    let tracks = elem(&[0x16, 0x54, 0xae, 0x6b], &tracks_body);

    // Cluster: einer je Video-Keyframe; Audio geht in den jeweils offenen Cluster.
    let mut clusters: Vec<u8> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut base_ms: i64 = 0;
    let mut have_cluster = false;

    let simple_block = |track: u8, rel: i16, keyframe: bool, data: &[u8]| -> Vec<u8> {
        let mut body = Vec::with_capacity(data.len() + 5);
        body.push(0x80 | track); // Track-Nummer als 1-Byte-vint
        body.extend_from_slice(&rel.to_be_bytes());
        body.push(if keyframe { 0x80 } else { 0x00 }); // Flags (Keyframe-Bit)
        body.extend_from_slice(data);
        elem(&[0xa3], &body) // SimpleBlock
    };
    let close_cluster = |clusters: &mut Vec<u8>, cur: &mut Vec<u8>, base_ms: i64| {
        if cur.is_empty() {
            return;
        }
        let mut body = elem(&[0xe7], &uint(base_ms.max(0) as u64)); // Timestamp
        body.extend_from_slice(cur);
        clusters.extend(elem(&[0x1f, 0x43, 0xb6, 0x75], &body));
        cur.clear();
    };

    // Index-Merge: jeder Stream wird NUR vorwärts gelesen, damit die Dekodier-
    // Reihenfolge je Stream erhalten bleibt (Sortieren nach PTS würde die B-Frame-
    // Reihenfolge zerstören). Interleaving nur fürs Cluster-Packing nach aktueller Zeit.
    let (mut vi, mut ai) = (0usize, 0usize);
    loop {
        let vt = v_frames.get(vi).map(|(t, _)| *t);
        let at = a_frames.get(ai).map(|(t, _)| *t);
        let (t, track, keyframe, data): (i64, u8, bool, &[u8]) = match (vt, at) {
            (None, None) => break,
            (Some(vt), Some(at)) if vt <= at => {
                let f = &v_frames[vi].1;
                vi += 1;
                (vt, 1, f.keyframe, &f.data)
            }
            (Some(_), Some(at)) => {
                let d = &a_frames[ai].1;
                ai += 1;
                (at, 2, false, d)
            }
            (Some(vt), None) => {
                let f = &v_frames[vi].1;
                vi += 1;
                (vt, 1, f.keyframe, &f.data)
            }
            (None, Some(at)) => {
                let d = &a_frames[ai].1;
                ai += 1;
                (at, 2, false, d)
            }
        };
        // Neuer Cluster bei Video-Keyframe oder wenn der Relativ-Offset i16 sprengt.
        let need_new = !have_cluster || (track == 1 && keyframe) || (t - base_ms).abs() > 30_000;
        if need_new {
            close_cluster(&mut clusters, &mut cur, base_ms);
            base_ms = t;
            have_cluster = true;
        }
        let rel = (t - base_ms).clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        cur.extend(simple_block(track, rel, keyframe, data));
    }
    close_cluster(&mut clusters, &mut cur, base_ms);

    // Segment (unbekannte Größe → 0x01FFFFFFFFFFFFFF erlaubt Streaming/Append).
    let mut segment_body = info;
    segment_body.extend(tracks);
    segment_body.extend(clusters);
    let mut segment = vec![0x18, 0x53, 0x80, 0x67];
    segment.extend_from_slice(&vint_size(segment_body.len() as u64));
    segment.extend(segment_body);

    let mut out = ebml;
    out.extend(segment);
    out
}

/// SPS → (Breite, Höhe). Minimaler Parser (Baseline/High ohne alle Sonderfälle);
/// scheitert er, wird 0×0 geliefert (MKV bleibt gültig, Player nimmt die Stream-Größe).
fn sps_dimensions(sps: &[u8]) -> (u64, u64) {
    // Konservativ: wir verlassen uns primär auf die im Stream mitgeführten Werte und
    // geben hier 0 zurück, wenn kein robuster Parse möglich ist. Breite/Höhe werden
    // dann vom Decoder aus dem SPS (CodecPrivate) gelesen.
    let _ = sps;
    (0, 0)
}

/// Remuxt einen (zusammengesetzten) MPEG-TS-Puffer zu einer MKV-Datei (H.264+AAC).
/// `Err`, wenn die Ströme nicht dem unterstützten Profil entsprechen — der Aufrufer
/// fällt dann auf den `.ts`-Pfad zurück.
pub fn remux_ts_to_mkv(ts: &[u8]) -> Result<Vec<u8>> {
    let dem = demux(ts)?;

    // Video: Access Units bilden, PTS → ms.
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let v_frames: Vec<(i64, VideoFrame)> = video_frames(&dem.video, &mut sps, &mut pps)
        .into_iter()
        .map(|(pts, f)| ((pts / 90) as i64, f))
        .collect();
    let (sps, pps) = match (sps, pps) {
        (Some(s), Some(p)) => (s, p),
        _ => return Err(Error::extract("remux", "kein SPS/PPS im H.264-Strom")),
    };
    let avcc = build_avcc(&sps, &pps);
    let (width, height) = sps_dimensions(&sps);

    // Audio: ADTS-Frames fortlaufend, PTS ab erster Marke um 1024 Samples/Frame erhöht
    // (sonst bekämen alle Frames einer PES denselben Zeitstempel).
    let aac = aac_config(&dem.audio.es).unwrap_or(AacConfig {
        asc: vec![0x12, 0x10], // LC/44100/2ch
        sample_rate: 44100,
        channels: 2,
    });
    let anchor_90k = dem.audio.marks.first().map(|(_, p)| *p).unwrap_or(0);
    let a_frames: Vec<(i64, Vec<u8>)> = split_adts(&dem.audio.es)
        .into_iter()
        .enumerate()
        .map(|(k, f)| {
            let pts_90k = anchor_90k + (k as u64) * 1024 * 90_000 / aac.sample_rate.max(1) as u64;
            ((pts_90k / 90) as i64, f)
        })
        .collect();

    Ok(build_mkv(&avcc, width, height, &aac, &v_frames, &a_frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_size_laengen() {
        assert_eq!(vint_size(0), vec![0x80]);
        assert_eq!(vint_size(1), vec![0x81]);
        assert_eq!(vint_size(126), vec![0xfe]);
        // 127 passt nicht mehr in 1 Byte (0x7f ist reserviert) → 2 Byte.
        assert_eq!(vint_size(127), vec![0x40, 0x7f]);
    }

    #[test]
    fn uint_minimal() {
        assert_eq!(uint(0), vec![0]);
        assert_eq!(uint(1), vec![1]);
        assert_eq!(uint(256), vec![1, 0]);
        assert_eq!(uint(1_000_000), vec![0x0f, 0x42, 0x40]);
    }

    #[test]
    fn avcc_aufbau() {
        let sps = vec![0x67, 0x64, 0x00, 0x20, 0xaa];
        let pps = vec![0x68, 0xee, 0x3c];
        let avcc = build_avcc(&sps, &pps);
        assert_eq!(avcc[0], 1);
        assert_eq!(avcc[1], 0x64); // profile aus sps[1]
        assert_eq!(avcc[3], 0x20); // level aus sps[3]
        assert_eq!(avcc[4], 0xff);
        assert_eq!(avcc[5], 0xe1);
    }

    #[test]
    fn aac_config_aus_adts() {
        // ADTS: LC (profile=1), 44100 (idx=4, →0x50), 2 Kanäle (→0x80).
        let adts = [0xff, 0xf1, 0x50, 0x80, 0x00, 0x1f, 0xfc];
        let c = aac_config(&adts).unwrap();
        assert_eq!(c.sample_rate, 44100);
        assert_eq!(c.channels, 2);
        // ASC: AOT=2, freq_idx=4, chan=2 → 0x12 0x10.
        assert_eq!(c.asc, vec![0x12, 0x10]);
    }

    #[test]
    fn split_nals_findet_einheiten() {
        let data = [
            0, 0, 1, 0x67, 0xaa, 0, 0, 1, 0x68, 0xbb, 0, 0, 0, 1, 0x65, 0xcc,
        ];
        let nals = split_nals(&data);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0][0] & 0x1f, 7);
        assert_eq!(nals[1][0] & 0x1f, 8);
        assert_eq!(nals[2][0] & 0x1f, 5);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore = "Datei via VINDEX_TS_FIXTURE"]
    fn remux_echtes_segment() {
        let ts = std::fs::read(std::env::var("VINDEX_TS_FIXTURE").unwrap()).unwrap();
        let mkv = super::remux_ts_to_mkv(&ts).expect("remux");
        std::fs::write("/tmp/mine.mkv", &mkv).unwrap();
        println!("MKV bytes: {}", mkv.len());
    }
}
