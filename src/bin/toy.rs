use core::mem;
use cranelift_jit_demo::jit;
use std::{env, fs};

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    assert!(args.len() > 1);

    let src = fs::read_to_string(&args[1]).expect("failed to read from file");

    let mut inputs: Vec<i64> = vec![];
    for input in &args[2..] {
        inputs.push(input.parse().unwrap());
    }

    // Create the JIT instance, which manages all generated functions and data.
    let mut jit = jit::JIT::default();

    match inputs.len() {
        0 => println!("{}", run_fun(&mut jit, &src, ())?),
        1 => println!("{}", run_fun(&mut jit, &src, inputs[0])?),
        2 => println!("{}", run_fun(&mut jit, &src, (inputs[0], inputs[1]))?),
        3 => println!("{}", run_fun(&mut jit, &src, (inputs[0], inputs[1], inputs[2]))?),
        _ => unimplemented!("four or more arguments"),
    }

    Ok(())
}

fn run_fun<I>(jit: &mut jit::JIT, code: &str, input: I) -> Result<i64, String> {
    unsafe { run_code(jit, code, input) }
}

/// Executes the given code using the Cranelift JIT compiler.
///
/// Feeds the given input into the JIT compiled function and returns the resulting output.
///
/// # Safety
///
/// This function is unsafe since it relies on the caller to provide it with the correct
/// input and output types. Using incorrect types at this point may corrupt the program's state.
unsafe fn run_code<I, O>(jit: &mut jit::JIT, code: &str, input: I) -> Result<O, String> {
    // JIT-compile function
    let code_fn = compile_code(jit, code)?;
    // And now we can call it!
    Ok(code_fn(input))
}

unsafe fn compile_code<I, O>(jit: &mut jit::JIT, code: &str) -> Result<fn(I) -> O, String> {
    // Pass the string to the JIT, and it returns a raw pointer to machine code.
    let code_ptr = jit.compile(code)?;
    // Cast the raw pointer to a typed function pointer. This is unsafe, because
    // this is the critical point where you have to trust that the generated code
    // is safe to be called.
    let code_fn = mem::transmute::<*const u8, fn(I) -> O>(code_ptr);
    Ok(code_fn)
}

/// Let's say hello, by calling into libc. The puts function is resolved by
/// dlsym to the libc function, and the string &hello_string is defined below.
#[allow(dead_code)]
const HELLO: &str = r#"
fn hello() -> (r) {
    puts(&hello_world)
}
"#;

/// The `(s)` declares a return variable; the function returns whatever value
/// it was assigned when the function exits. Note that there are multiple
/// assignments, so the input is not in SSA form, but that's ok because
/// Cranelift handles all the details of translating into SSA form itself.
#[allow(dead_code)]
const SUM: &str = r#"
fn sum(n) -> (s) {
    s = 0
    i = 1
    while i <= n {
        s = s + i
        i = i + 1
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn run_hello(jit: &mut jit::JIT) -> Result<i64, String> {
        jit.create_data("hello_world", "Hello, World!\0".as_bytes().to_vec())?;
        run_fun(jit, HELLO, ())
    }

    #[test]
    fn hello() {
        let mut jit = jit::JIT::default();
        assert_eq!(run_hello(&mut jit), Ok(0));
    }

    #[test]
    fn sum() {
        let mut jit = jit::JIT::default();
        let f = unsafe { compile_code::<i64, i64>(&mut jit, SUM) }.unwrap();
        for i in 1..=10 {
            assert_eq!(f(i), i * (i + 1) / 2);
        }
    }
}
