use crate::GlommioCt;
use glommio_crate::LocalExecutorBuilder;

pub fn new_glommio_ct(name: &str, cpu_set: Option<usize>) -> GlommioCt {
    let mut builder = LocalExecutorBuilder::new().name(&name);
    if let Some(binding) = cpu_set {
        builder = builder.pin_to_cpu(binding);
    }
    GlommioCt::new(builder).unwrap()
}
