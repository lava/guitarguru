use cpal::traits::{DeviceTrait, HostTrait};
fn main() {
    for hid in cpal::available_hosts() {
        let host = cpal::host_from_id(hid).unwrap();
        println!("== host {:?}", hid);
        for d in host.input_devices().unwrap() {
            let desc = d.description().ok();
            println!("IN id={:?} name={:?} ext={:?}", d.id().ok().map(|i| i.to_string()), desc.as_ref().map(|x| x.name().to_owned()), desc.as_ref().map(|x| x.extended().map(|s| s.to_owned()).collect::<Vec<_>>()));
        }
        for d in host.output_devices().unwrap() {
            println!("OUT id={:?} name={:?}", d.id().ok().map(|i| i.to_string()), d.description().ok().map(|x| x.name().to_owned()));
        }
        println!("default in {:?}", host.default_input_device().and_then(|d| d.id().ok()).map(|i| i.to_string()));
    }
}
