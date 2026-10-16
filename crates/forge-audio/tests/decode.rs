//! Decoding and streaming, on sounds made in code (issue #267, scope 1).

mod common;

use std::io::Cursor;

use common::*;
use forge_audio::decode::{Decoder, WavDecoder, WavSampleFormat, decode_all, open};
use forge_audio::sound::{noise, sine, sine_loop, stereo};
use forge_audio::stream::streams;
use forge_audio::{
    BusLayout, MixerConfig, OfflineRenderer, PlayParams, WavFormat, create, wav_bytes,
};

const RATE: u32 = 48_000;

#[test]
fn wav_round_trips_in_every_format() {
    let left = sine(440.0, 0.8, 0.25, 44_100);
    let right = noise(5, 0.5, 0.25, 44_100);
    let sound = stereo(&left, &right);
    for (format, tolerance, stored) in [
        (WavFormat::Pcm16, 1.0 / 32_000.0, WavSampleFormat::I16),
        (WavFormat::Pcm24, 1.0 / 8_000_000.0, WavSampleFormat::I24),
        (WavFormat::Float32, 0.0, WavSampleFormat::F32),
    ] {
        let bytes = wav_bytes(sound.samples(), 2, 44_100, format);
        let mut decoder = WavDecoder::new(Cursor::new(bytes)).expect("a WAV header");
        assert_eq!(decoder.info().format, stored);
        assert_eq!(decoder.frames(), Some(sound.frames() as u64));
        let decoded = decode_all(&mut decoder).expect("decoded");
        assert_eq!(
            (decoded.rate(), decoded.channels(), decoded.frames()),
            (44_100, 2, sound.frames())
        );
        let worst = decoded
            .samples()
            .iter()
            .zip(sound.samples())
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst <= tolerance, "{format:?}: off by {worst}");
    }
}

/// A WAV file with a `LIST` chunk of odd length before its data, 8-bit or 32-bit integers.
fn unusual_wav(bits: u16, samples: &[f32]) -> Vec<u8> {
    let bytes = bits as usize / 8;
    let mut data = Vec::new();
    for &x in samples {
        match bits {
            8 => data.push(((x * 127.0).round() as i16 + 128) as u8),
            32 => {
                data.extend_from_slice(&((x as f64 * 2_147_483_647.0).round() as i32).to_le_bytes())
            }
            _ => unreachable!(),
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * bytes as u32).to_le_bytes());
    out.extend_from_slice(&(bytes as u16).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(b"abc\0");
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

#[test]
fn wav_reads_past_other_chunks_and_other_widths() {
    let tone = sine(1000.0, 0.5, 0.05, RATE);
    for (bits, tolerance) in [(8u16, 1.0 / 100.0), (32, 1e-7)] {
        let bytes = unusual_wav(bits, tone.samples());
        let decoded =
            decode_all(open(Cursor::new(bytes)).expect("a decoder").as_mut()).expect("decoded");
        assert_eq!(decoded.frames(), tone.frames());
        let worst = decoded
            .samples()
            .iter()
            .zip(tone.samples())
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst <= tolerance, "{bits} bits: off by {worst}");
    }
}

#[test]
fn a_streamed_sound_plays_as_the_same_sound_from_memory() {
    // Stereo at 44.1 kHz into a 48 kHz mixer: the resampler runs on both paths.
    let sound = stereo(&sine(300.0, 0.4, 0.6, 44_100), &noise(9, 0.3, 0.6, 44_100));
    let bytes = wav_bytes(sound.samples(), 2, 44_100, WavFormat::Float32);
    let render = |streamed: bool| {
        let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
        let (opener, pump) = streams(0.1);
        let params = PlayParams {
            pitch: 1.1,
            ..PlayParams::default()
        };
        if streamed {
            let decoder = open(Cursor::new(bytes.clone())).expect("a decoder");
            let stream = opener.open(decoder, false).expect("a stream");
            engine.play_stream(stream, params);
        } else {
            engine.play(&sound.clone().shared(), params);
        }
        let mut renderer = OfflineRenderer::with_streams(mixer, pump);
        let out = renderer.render_seconds(0.8);
        let events = engine.update().to_vec();
        (out, events, engine.stats())
    };
    let (memory, memory_events, _) = render(false);
    let (streamed, streamed_events, stats) = render(true);
    assert!(rms(&memory) > 0.05);
    assert!(
        memory
            .iter()
            .zip(&streamed)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    assert_eq!(memory_events.len(), 1);
    assert_eq!(memory_events, streamed_events);
    assert_eq!(stats.underruns, 0);
}

#[test]
fn a_looping_stream_loops_like_a_looping_sound() {
    let sound = sine_loop(500.0, 0.5, 0.05, RATE);
    let bytes = wav_bytes(sound.samples(), 1, RATE, WavFormat::Float32);
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let (opener, pump) = streams(0.05);
    let stream = opener
        .open(open(Cursor::new(bytes)).expect("a decoder"), true)
        .expect("a stream");
    let looping = PlayParams {
        looping: true,
        pitch: 0.97,
        ..PlayParams::default()
    };
    engine.play_stream(stream, looping);
    engine.play(
        &sound.shared(),
        PlayParams {
            pan: 1.0,
            ..looping
        },
    );
    // The stream on the left (and half on the right), the sound in memory on the right.
    let mut renderer = OfflineRenderer::with_streams(mixer, pump);
    let out = renderer.render_seconds(0.5);
    let (left, right) = (channel(&out, 0), channel(&out, 1));
    let c = std::f32::consts::FRAC_1_SQRT_2;
    let worst = left
        .iter()
        .zip(&right)
        .skip(1000)
        .fold(0.0f32, |m, (&l, &r)| m.max((l / c - (r - l)).abs()));
    assert!(worst < 1e-4, "the loops part by {worst}");
    assert!(rms(&left[1000..]) > 0.2);
}

#[test]
fn a_starved_stream_plays_silence_and_counts_it() {
    let sound = noise(2, 0.5, 2.0, RATE);
    let bytes = wav_bytes(sound.samples(), 1, RATE, WavFormat::Float32);
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let (opener, _pump) = streams(0.1);
    let stream = opener
        .open(open(Cursor::new(bytes)).expect("a decoder"), false)
        .expect("a stream");
    assert_eq!(stream.buffered(), 4800);
    engine.play_stream(stream, PlayParams::default());
    // Nobody pumps: the ring's 0.1 s plays, then silence.
    let mut renderer = OfflineRenderer::new(mixer);
    let left = channel(&renderer.render_seconds(0.5), 0);
    assert!(rms(&left[256..4800]) > 0.1);
    assert!(left[5200..].iter().all(|&v| v == 0.0));
    assert!(engine.stats().underruns > 0);
    assert!(engine.update().is_empty(), "a starved stream does not end");
}

/// `tests/data/tone-vorbis.ogg`: the tone `audio-render tone` writes (440 Hz at 0.5 on the
/// left, 1000 Hz at 0.25 on the right, 1 s at 48 kHz) encoded by libvorbis through FFmpeg
/// (Blender 5.2's audaspace, `Sound.write`, 128 kbit/s).
const VORBIS: &[u8] = include_bytes!("data/tone-vorbis.ogg");

/// The tone the fixtures were made from.
fn fixture_source() -> forge_audio::SampleBuffer {
    stereo(&sine(440.0, 0.5, 1.0, RATE), &sine(1000.0, 0.25, 1.0, RATE))
}

/// Checks a decoded fixture against its source: length, levels, crosstalk, and the error.
fn check_against_source(decoded: &forge_audio::SampleBuffer, snr_db: f64) {
    let source = fixture_source();
    assert_eq!((decoded.channels(), decoded.rate()), (2, RATE));
    assert_eq!(decoded.frames(), source.frames());
    let middle = 12_000..36_000;
    for (c, frequency, level, other) in [(0, 440.0, 0.5, 1000.0), (1, 1000.0, 0.25, 440.0)] {
        let heard = channel(decoded.samples(), c);
        let made = channel(source.samples(), c);
        let measured = tone(&heard[middle.clone()], frequency, RATE);
        assert!(
            (measured / level - 1.0).abs() < 0.02,
            "channel {c}: {measured} for {level}"
        );
        assert!(
            tone(&heard[middle.clone()], other, RATE) < 0.002,
            "crosstalk on channel {c}"
        );
        let error: Vec<f32> = heard[middle.clone()]
            .iter()
            .zip(&made[middle.clone()])
            .map(|(a, b)| a - b)
            .collect();
        let snr = db(rms(&made[middle.clone()]) / rms(&error));
        assert!(snr > snr_db, "channel {c}: {snr:.1} dB against the source");
    }
}

#[test]
fn ogg_vorbis_decodes_to_the_tone_it_was_made_from() {
    let mut decoder = open(Cursor::new(VORBIS.to_vec())).expect("a Vorbis decoder");
    assert_eq!(
        (decoder.channels(), decoder.sample_rate(), decoder.frames()),
        (2, RATE, None)
    );
    let decoded = decode_all(decoder.as_mut()).expect("decoded");
    check_against_source(&decoded, 30.0);
    // Rewound, it gives the same samples again.
    decoder.rewind().expect("rewound");
    assert_eq!(
        decode_all(decoder.as_mut()).expect("decoded again"),
        decoded
    );
}

#[test]
fn ogg_vorbis_streams_as_it_decodes() {
    let decoded = decode_all(
        open(Cursor::new(VORBIS.to_vec()))
            .expect("a decoder")
            .as_mut(),
    )
    .expect("decoded");
    let render = |streamed: bool| {
        let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
        let (opener, pump) = streams(0.05);
        if streamed {
            let stream = opener
                .open(
                    open(Cursor::new(VORBIS.to_vec())).expect("a decoder"),
                    false,
                )
                .expect("a stream");
            engine.play_stream(stream, PlayParams::default());
        } else {
            engine.play(&decoded.clone().shared(), PlayParams::default());
        }
        OfflineRenderer::with_streams(mixer, pump).render_seconds(1.2)
    };
    let memory = render(false);
    assert!(rms(&memory) > 0.1);
    assert!(
        memory
            .iter()
            .zip(&render(true))
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

/// `tests/data/tone-opus.ogg`: the same tone encoded by libopus through FFmpeg (Blender
/// 5.2's audaspace, 64 kbit/s; pre-skip 312).
const OPUS: &[u8] = include_bytes!("data/tone-opus.ogg");

#[test]
fn ogg_opus_decodes_to_the_tone_it_was_made_from() {
    let mut decoder = open(Cursor::new(OPUS.to_vec())).expect("an Opus decoder");
    assert_eq!((decoder.channels(), decoder.sample_rate()), (2, RATE));
    let decoded = decode_all(decoder.as_mut()).expect("decoded");
    check_against_source(&decoded, 20.0);
    decoder.rewind().expect("rewound");
    assert_eq!(
        decode_all(decoder.as_mut()).expect("decoded again"),
        decoded
    );
}

#[test]
fn ogg_opus_loops_as_a_stream() {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let (opener, pump) = streams(0.1);
    let stream = opener
        .open(open(Cursor::new(OPUS.to_vec())).expect("a decoder"), true)
        .expect("a stream");
    engine.play_stream(
        stream,
        PlayParams {
            looping: true,
            ..PlayParams::default()
        },
    );
    let out = OfflineRenderer::with_streams(mixer, pump).render_seconds(2.5);
    // Three passes of a one-second file: the 440 Hz tone stays on the left throughout.
    let left = channel(&out, 0);
    for second in 0..2 {
        let window = &left[second * 48_000 + 12_000..second * 48_000 + 36_000];
        let level = tone(window, 440.0, RATE);
        // A stereo sound keeps its sides at unity gain (balance, not the mono pan law).
        assert!((level / 0.5 - 1.0).abs() < 0.03, "second {second}: {level}");
    }
    assert_eq!(engine.stats().underruns, 0);
}
