use olio_audio::{Clip, Peaks};

#[test]
fn each_slice_holds_its_lowest_and_highest_sample() {
    let clip = Clip::new(8, 1, vec![0.0, 1.0, -1.0, 0.5]);
    assert_eq!(Peaks::of(&clip, 2).slices(), [(0.0, 1.0), (-1.0, 0.5)]);
}

#[test]
fn peaks_cover_every_channel() {
    let clip = Clip::new(8, 2, vec![0.1, -0.7, 0.3, 0.2]);
    assert_eq!(Peaks::of(&clip, 1).slices(), [(-0.7, 0.3)]);
}

#[test]
fn a_short_clip_gets_one_slice_per_frame() {
    let clip = Clip::new(8, 1, vec![0.1, 0.2, 0.3]);
    let peaks = Peaks::of(&clip, 100);
    assert_eq!(peaks.len(), 3);
    assert_eq!(peaks.slices()[1], (0.2, 0.2));
}

#[test]
fn columns_regroup_slices_for_drawing() {
    let clip = Clip::new(8, 1, vec![0.1, -0.2, 0.3, -0.4]);
    let peaks = Peaks::of(&clip, 4);
    assert_eq!(peaks.columns(2), [(-0.2, 0.1), (-0.4, 0.3)]);
    assert_eq!(peaks.columns(8).len(), 8);
    assert_eq!(peaks.columns(8)[1], (0.1, 0.1));
    assert!(peaks.columns(0).is_empty());
    assert!(Peaks::default().columns(10).is_empty());
}

#[test]
fn columns_can_cover_part_of_a_clip() {
    let clip = Clip::new(8, 1, vec![0.1, -0.2, 0.3, -0.4]);
    let peaks = Peaks::of(&clip, 4);
    assert_eq!(
        peaks.columns_between(2, 0.5, 1.0),
        [(0.3, 0.3), (-0.4, -0.4)]
    );
    assert_eq!(peaks.columns_between(1, 0.25, 0.5), [(-0.2, -0.2)]);
    assert_eq!(peaks.columns_between(1, 0.9, 0.9).len(), 1);
}
