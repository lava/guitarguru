use cpal::traits::{DeviceTrait, HostTrait};
fn main() {
    let host = cpal::default_host();
    for d in host.input_devices().unwrap() {
        let name = d.description().map(|x| format!("{:?}", x)).unwrap_or_default();
        let cfgs: Vec<String> = d.supported_input_configs().map(|c| c.map(|c| format!("{}ch {:?} {}-{} {:?}", c.channels(), c.sample_format(), c.min_sample_rate(), c.max_sample_rate(), c.buffer_size())).collect()).unwrap_or_default();
        println!("IN {name}\n   {cfgs:?}");
    }
    for d in host.output_devices().unwrap() {
        println!("OUT {:?}", d.description().map(|x| x.name().to_owned()));
    }
    println!("default in {:?}", host.default_input_device().and_then(|d| d.description().ok()).map(|d| d.name().to_owned()));
}
