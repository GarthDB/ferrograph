// `helper` is bound by the second `use`, not declared in this module.
use helper::build;

use crate::a::helper;

pub fn api_via_binding() {
    build();
}
