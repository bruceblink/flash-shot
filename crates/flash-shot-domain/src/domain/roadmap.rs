//! Initial delivery roadmap shown by the application shell.

/// A product capability tracked by the roadmap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capability {
    pub name: &'static str,
    pub description: &'static str,
}

/// A bounded delivery phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryPhase {
    pub name: &'static str,
    pub outcome: &'static str,
    pub capabilities: &'static [Capability],
}

/// Product roadmap independent from the GPUI presentation layer.
#[derive(Clone, Copy, Debug)]
pub struct ProductRoadmap {
    phases: &'static [DeliveryPhase],
}

const FOUNDATION: &[Capability] = &[
    Capability {
        name: "Native shell",
        description: "GPUI window, logging, single instance, tray, and global shortcut.",
    },
    Capability {
        name: "Performance harness",
        description: "Measure startup, capture latency, frame time, memory, and resource growth.",
    },
];

const CAPTURE: &[Capability] = &[
    Capability {
        name: "Windows capture",
        description: "Capture monitors and windows with correct mixed-DPI coordinates.",
    },
    Capability {
        name: "Selection overlay",
        description: "Low-latency fullscreen selection with copy, save, and cancel actions.",
    },
];

const ANNOTATION: &[Capability] = &[
    Capability {
        name: "Annotation document",
        description: "Engine-neutral shapes, commands, hit testing, and undo/redo.",
    },
    Capability {
        name: "Screenshot tools",
        description: "Rectangle, ellipse, arrow, pen, text, blur, highlight, and numbering.",
    },
];

const ADVANCED: &[Capability] = &[
    Capability {
        name: "Productivity",
        description: "Pinning, history, OCR, QR recognition, and translation.",
    },
    Capability {
        name: "Advanced capture",
        description: "Scrolling screenshots and FFmpeg-based screen recording.",
    },
];

const PHASES: &[DeliveryPhase] = &[
    DeliveryPhase {
        name: "Foundation",
        outcome: "A measurable and reliable native desktop shell.",
        capabilities: FOUNDATION,
    },
    DeliveryPhase {
        name: "Capture MVP",
        outcome: "A fast end-to-end screenshot workflow on Windows.",
        capabilities: CAPTURE,
    },
    DeliveryPhase {
        name: "Annotation",
        outcome: "A focused native editor for screenshot markup.",
        capabilities: ANNOTATION,
    },
    DeliveryPhase {
        name: "Advanced workflows",
        outcome: "The most useful Snow Shot workflows without its WebView architecture.",
        capabilities: ADVANCED,
    },
];

impl ProductRoadmap {
    pub const fn current() -> Self {
        Self { phases: PHASES }
    }

    pub const fn phases(&self) -> &'static [DeliveryPhase] {
        self.phases
    }
}

#[cfg(test)]
mod tests {
    use super::ProductRoadmap;

    #[test]
    fn roadmap_has_bounded_non_empty_phases() {
        let roadmap = ProductRoadmap::current();

        assert!(!roadmap.phases().is_empty());
        assert!(
            roadmap
                .phases()
                .iter()
                .all(|phase| !phase.capabilities.is_empty())
        );
    }

    #[test]
    fn capture_mvp_precedes_annotation_and_advanced_workflows() {
        let names: Vec<_> = ProductRoadmap::current()
            .phases()
            .iter()
            .map(|phase| phase.name)
            .collect();

        assert_eq!(
            names,
            [
                "Foundation",
                "Capture MVP",
                "Annotation",
                "Advanced workflows"
            ]
        );
    }
}
