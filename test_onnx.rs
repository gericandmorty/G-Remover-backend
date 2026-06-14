fn main() {
    let session = ort::session::Session::builder().unwrap().commit_from_file("assets/rmbg-1.4.onnx").unwrap();
    for input in session.inputs {
        println!("Input: {} - {:?}", input.name, input.input_type);
    }
}
