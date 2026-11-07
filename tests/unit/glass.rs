use super::*;

#[test]
fn glass_profile_is_a_static_backdrop_lens_not_an_opaque_cover() {
    let layer = Fx::lens(lens(0.04)).on(Shape::rounded(0.0, 0.0, 180.0, 50.0, 12.0));
    assert!(layer.fx.needs_backdrop());
    assert!(!layer.is_animating());
    assert_eq!(lens(0.04).tint_amount, 0.04);
    assert_eq!(lens(-1.0).tint_amount, 0.0);
    assert_eq!(lens(2.0).tint_amount, 1.0);
    assert!(lens(0.04).frost > 0.0);
    assert_eq!(lens(0.04).dispersion, 0.0);
    let material = lens(0.04);
    assert!(material.bevel <= 4.0 && material.depth <= 6.0);
    assert!(material.rim <= 0.5 && material.shadow <= 0.18);
    assert!(material.magnification <= 1.005);
    assert!(material.frost >= 0.25);
    assert_eq!(material.tint, rgb(0xEDF1F5));
}
