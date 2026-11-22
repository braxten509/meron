// Imports Meron's native C interface into the iOS
// Swift target. Xcode uses this bridging header
// so the host can call the exported Rust
// functions.

#import "../RustCore/meron_core.h"
