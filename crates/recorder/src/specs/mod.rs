//! The fixture catalogue: every read method from PLAN.md section 7 with its variants, plus the
//! error cases.

mod catalog;
mod errors;
mod global;
mod user;

use crate::discover::Discovered;
use crate::spec::Spec;

/// Every fixture to record, in order.
pub(crate) fn all(d: &Discovered) -> Vec<Spec> {
    [
        user::specs(d),
        catalog::specs(d),
        global::specs(d),
        errors::specs(d),
    ]
    .concat()
}

/// The same request at several page sizes and pages: limits 1, 50 and 200, pages 2 and last.
/// The last page is counted at the default size of 50.
fn paging(base: &Spec, last_page: u32) -> Vec<Spec> {
    vec![
        base.with_variant("limit-1").num("limit", 1),
        base.with_variant("limit-50").num("limit", 50),
        base.with_variant("limit-200").num("limit", 200),
        base.with_variant("page-2").num("page", 2),
        base.with_variant("page-last")
            .num("page", u64::from(last_page.max(1))),
    ]
}
