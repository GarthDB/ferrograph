mod complete;

pub(crate) use crate::flags::complete::{
    bash::generate as generate_bash, fish::generate as generate_fish,
};
