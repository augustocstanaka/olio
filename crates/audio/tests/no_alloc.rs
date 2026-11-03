//! The Generator's render never allocates: a counting allocator watches
//! a minute of every layer playing, synthesised and then from recordings
//! (with a recording replaced while it plays), after a moment to settle.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use std::path::Path;
use std::sync::Arc;

use olio_audio::ambience::{Ambience, Instrument, Journey, Style};
use olio_audio::instruments::{
    InstrumentManifest, MANIFEST, Manifest, Multisample, PACK_VERSION, Pack, ZoneManifest,
};
use olio_audio::{Clip, WavFormat};

struct Counting;

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    if WATCHING.with(Cell::get) {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A pack of sine waves: struck ones for the harp, piano and bells, and
/// looped ones for the strings and the pad, so held notes carry on.
fn write_pack(dir: &Path) {
    let rate = 44_100;
    let tone = |hz: f32, seconds: f32, decay: bool| {
        let frames = (seconds * rate as f32) as usize;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let envelope = if decay { (-t * 2.0).exp() } else { 1.0 };
                let s = (t * hz * std::f32::consts::TAU).sin() * 0.4 * envelope;
                [s, s]
            })
            .collect();
        Clip::new(rate, 2, samples)
    };
    let mut instruments = Vec::new();
    for (instrument, name, held) in [
        (Instrument::Harp, "harp", false),
        (Instrument::Piano, "piano", false),
        (Instrument::Bells, "bells", false),
        (Instrument::Strings, "strings", true),
        (Instrument::Pad, "pad", true),
    ] {
        let file = format!("{name}.wav");
        tone(220.0, 2.0, !held)
            .write_wav(&dir.join(&file), WavFormat::Int16)
            .unwrap();
        instruments.push(InstrumentManifest {
            instrument,
            attack: 0.05,
            release: 0.5,
            gain: 1.0,
            zones: vec![ZoneManifest {
                file,
                root: 57,
                low: 0,
                high: 127,
                velocity: (0.0, 1.0),
                // Whole cycles of 220 Hz: a seamless loop.
                looped: held.then_some((22_050 - 4_410, 22_050 + 4_410 * 3)),
                gain: 1.0,
                pitched: true,
                tune: 0.0,
            }],
        });
    }
    let manifest = Manifest {
        version: PACK_VERSION,
        instruments,
    };
    std::fs::write(
        dir.join(MANIFEST),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
}

/// Renders `ambience` for a second to settle, then a minute watched,
/// calling `during` once in the middle; how many times it allocated.
fn watched(ambience: &mut Ambience, mut during: impl FnMut(&mut Ambience)) -> usize {
    let rate = 48_000;
    let mut out = vec![0.0f32; 512 * 2];
    for _ in 0..rate / 512 {
        ambience.render(&mut out, 2, false);
    }
    let blocks = rate * 60 / 512;
    WATCHING.with(|w| w.set(true));
    for block in 0..blocks {
        if block == blocks / 2 {
            during(ambience);
        }
        ambience.render(&mut out, 2, block % 700 < 100);
    }
    WATCHING.with(|w| w.set(false));
    ALLOCATIONS.with(|n| n.replace(0))
}

#[test]
fn rendering_the_generator_never_allocates() {
    let rate = 48_000;
    let dir = tempfile::tempdir().unwrap();
    write_pack(dir.path());
    let pack = Pack::open(dir.path()).unwrap();
    let load =
        |instrument| -> Option<Arc<Multisample>> { pack.load(instrument).unwrap().map(Arc::new) };
    let recorded: Vec<(Instrument, Arc<Multisample>)> = pack
        .instruments()
        .filter_map(|instrument| Some((instrument, load(instrument)?)))
        .collect();
    for (style, journey) in [
        (Style::OpenRoad, Journey::Steady),
        (Style::Cathedral, Journey::Evolve),
        (Style::RainStudy, Journey::Build),
    ] {
        let mut settings = style.settings();
        settings.enabled = true;
        settings.journey = journey;
        for layer in &mut settings.layers {
            layer.enabled = true;
        }
        let mut ambience = Ambience::new(rate as u32, 4);
        ambience.set(settings);
        let synthesised = watched(&mut ambience, |_| {});
        assert_eq!(
            synthesised,
            0,
            "{} allocated {synthesised} times",
            style.name()
        );

        let mut ambience = Ambience::new(rate as u32, 4);
        ambience.set(settings);
        for (instrument, samples) in &recorded {
            ambience.set_samples(*instrument, Some(Arc::clone(samples)));
        }
        let mut replacement = load(Instrument::Pad);
        let allocations = watched(&mut ambience, |ambience| {
            // The pad's recording replaced while it plays: the old one
            // retires, to be taken and freed later, off this thread.
            ambience.set_samples(Instrument::Pad, replacement.take());
        });
        assert_eq!(
            allocations,
            0,
            "{} from recordings allocated {allocations} times",
            style.name()
        );
        // Freed here, as the app frees them, off the audio thread.
        while ambience.take_released().is_some() {}
    }
}
