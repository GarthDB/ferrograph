mod real {
    pub fn install() {}
}
mod stub {
    pub fn install() {}
}
use real as platform;

pub fn api_module_alias() {
    platform::install();
}
