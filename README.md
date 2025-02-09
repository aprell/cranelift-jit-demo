Hello!

This is a simple demo that JIT-compiles a toy language, using Cranelift.

It uses the new JIT interface in development
[here](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift/jit). JIT takes care
of managing a symbol table, allocating memory, and performing relocations, offering
a relatively simple API.

This is inspired in part by Ulysse Carion's
[llvm-rust-getting-started](https://github.com/ucarion/llvm-rust-getting-started)
and JT's [rustyjit](https://github.com/jntrnr/rustyjit).

A quick introduction to Cranelift: Cranelift is a compiler backend. It's
light-weight, supports `no_std` mode, doesn't use floating-point itself,
and it makes efficient use of memory.

Cranelift is being architected to allow flexibility in how one uses it.
Sometimes that flexibility can be a burden, which we've recently started to
address in a new set of crates, `cranelift-module`, `cranelift-jit`, and
`cranelift-faerie`, which put the pieces together in some easy-to-use
configurations for working with multiple functions at once. `cranelift-module`
is a common interface for working with multiple functions and data interfaces at
once. This interface can sit on top of `cranelift-jit`, which writes code and
data to memory where they can be executed and accessed. And it can sit on top of
`cranelift-faerie`, which writes code and data to native object files which can
be linked into native executables.

This post introduces Cranelift by walking through a JIT demo, using the
[`cranelift-jit`](https://crates.io/crates/cranelift-jit) crate. Currently,
this demo works on Linux x86-64 platforms. It may also work on Mac x86-64
platforms, though I haven't specifically tested that yet. Cranelift is being
designed to support many other kinds of platforms in the future.

### A walkthrough

First, let's take a quick look at the toy language in use. It's a very simple
language, in which all variables have type `isize`. (Cranelift does have full
support for other integer and floating-point types, so this is just to keep the
toy language simple).

For a quick flavor, here's our [first example](./examples/foo.txt) in the toy
language:

```
fn foo(a, b) -> (c) {
    if a {
        if b {
            c = 30
        } else {
            c = 40
        }
    } else {
        c = 50
    }
    c = c + 2
}
```

The grammar for this toy language is defined [here](./src/frontend.rs#L23), and
this demo uses the [peg](https://crates.io/crates/peg) parser generator library
to generate actual parser code for it.

The output of parsing is a [custom AST type](./src/frontend.rs#L1):

```rust
pub enum Expr {
    Literal(String),
    Identifier(String),
    Assign(String, Box<Expr>),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    IfElse(Box<Expr>, Vec<Expr>, Vec<Expr>),
    WhileLoop(Box<Expr>, Vec<Expr>),
    Call(String, Vec<Expr>),
    GlobalDataAddr(String),
}
```

It's pretty minimal and straightforward. The `IfElse` can return a value, to
show how that's done in Cranelift (see below).

The [first thing we do](./src/bin/toy.rs#L17) is create an instance of our `JIT`:

```rust
let mut jit = jit::JIT::default();
```

The `JIT` class is defined [here](./src/jit.rs#L9) and contains several fields:

 - `builder_context` - Cranelift uses this function builder context to reuse
   dynamic allocations between compiling multiple functions.
 - `ctx` - This is the main `Context` object for compiling functions.
 - `data_description` - Similar to `ctx`, but for "compiling" data sections.
 - `module` - The `Module` which holds information about all functions and data
   objects defined in the current `JIT`.

Before we go any further, let's talk about the underlying model here. The
`Module` class divides the world into two kinds of things: functions and data
objects. Both functions and data objects have *names*, and can be imported into
a module, defined and only referenced locally, or defined and exported for use
in outside code. Functions are immutable, while data objects can be declared
either read-only or writable.

Both functions and data objects can contain references to other functions and
data objects. Cranelift is designed to allow the low-level parts operate on each
function and data object independently, so each function and data object
maintains its own individual namespace of imported names. The
[`Module`](https://docs.rs/cranelift-module/latest/cranelift_module/trait.Module.html)
struct takes care of maintaining a set of declarations for use across multiple
functions and data objects.

These concepts are sufficiently general that they're applicable to JITing as
well as native object files (more discussion below), and `Module` provides an
interface which abstracts over both.

Once we've [initialized the JIT data structures](./src/jit.rs#L28), we use
our `JIT` to [compile](./src/jit.rs#L52) some functions.

The `JIT`'s `compile` function takes a string containing a function in the toy
language. It [parses](./src/jit.rs#L55) the string into an AST and then
[translates](./src/jit.rs#L58) the AST into Cranelift IR.

Our toy language only supports one type, so we start by [declaring that
type](./src/jit.rs#L125) for convenience.

We then start translating the function by adding [the function
parameters](./src/jit.rs#L127) and [return types](./src/jit.rs#L133) to the
Cranelift function signature.

We [create](./src/jit.rs#L136) a
[FunctionBuilder](https://docs.rs/cranelift-frontend/latest/cranelift_frontend/struct.FunctionBuilder.html),
which is a utility for building up the contents of a Cranelift IR function. As
we'll see below, `FunctionBuilder` includes functionality for constructing SSA
form automatically so that users don't have to worry about it.

Next, we [start](./src/jit.rs#L139) an initial basic block (block), which is the
entry block of the function and the place where we'll insert some code. A basic
block is a sequence of IR instructions with a single entry point and no branches
until the end, so execution always starts at the top and proceeds straight
through to the end.

Cranelift's basic blocks can have parameters. These take the place of PHI
functions in other IRs.

Here's an example of a block with some parameters and branches (`brif` and
`jump`) at the end of the block:

```
block0(v0: i32, v1: i32, v2: i32, v507: i64):
    v508 = iconst.i32 0
    v509 = iconst.i64 0
    v404 = ifcmp_imm v2, 0
    v10 = iadd_imm v2, -7
    v405 = ifcmp_imm v2, 7
    brif ugt v405, block29(v10)
    jump block29(v508)
```

The `FunctionBuilder` library will take care of inserting block parameters
automatically, so frontends that don't need to use them directly generally don't
need to worry about them, though one place they do come up is that function
arguments are represented as block parameters of the entry block. We must tell
Cranelift to add the parameters, using
[`append_block_params_for_function_params`](https://docs.rs/cranelift-frontend/latest/cranelift_frontend/struct.FunctionBuilder.html#method.append_block_params_for_function_params)
like [so](./src/jit.rs#L145).

The `FunctionBuilder` keeps track of a "current" block into which new
instructions are inserted. We [inform](./src/jit.rs#L148) the `FunctionBuilder`
of our new block, using
[`switch_to_block`](https://docs.rs/cranelift-frontend/latest/cranelift_frontend/struct.FunctionBuilder.html#method.switch_to_block)
so that we can start inserting instructions.

A major concept about blocks is that the `FunctionBuilder` wants to know when
all branches that could branch to a block have been seen, at which point the
block can be *sealed*, enabling SSA construction. All blocks must be sealed by
the end of the function. We [seal](./src/jit.rs#L153) a block with
[`seal_block`](https://docs.rs/cranelift-frontend/latest/cranelift_frontend/struct.FunctionBuilder.html#method.seal_block).

Our toy language doesn't have explicit variable declarations, so we walk the AST
to discover all variables to be able to [declare](./src/jit.rs#L158) them to the
`FunctionBuilder`. These variables need not be in SSA form; the
`FunctionBuilder` will take care of constructing SSA form internally.

For convenience when walking the function body, the demo here
[uses](./src/jit.rs#L161) a `FunctionTranslator` object, which holds the
`FunctionBuilder`, the current `Module`, as well as the symbol table for looking
up variables. Now we can start [walking the function body](./src/jit.rs#L168).

[AST translation](./src/jit.rs#L196) utilizes the instruction-building features
of `FunctionBuilder`. Let's start with a simple example translating integer
literals:

```rust
Expr::Literal(literal) => {
    let imm: i32 = literal.parse().unwrap();
    self.builder.ins().iconst(self.int, i64::from(imm))
}
```

The first part is just extracting the integer value from the AST. The next line
is the builder line:

 - The `.ins()` returns an "insertion object", which allows inserting an
   instruction at the end of the currently active block.
 - `iconst` is the name of the builder routine for creating [integer
   constants](https://docs.rs/cranelift-codegen/latest/cranelift_codegen/ir/trait.InstBuilder.html#method.iconst)
   in Cranelift. Every instruction in the IR can be created directly through
   such a function call.

Translation of [Add](./src/jit.rs#L205)s and other arithmetic operations is
similarly straightforward.

Translation of [variable references](./src/jit.rs#L237) is mostly handled by
`FunctionBuilder`'s `use_var` function:

```rust
Expr::Identifier(name) => {
    // `use_var` is used to read the value of a variable.
    let variable = self.variables.get(&name).expect("variable not defined");
    self.builder.use_var(*variable)
}
```

`use_var` is for reading the value of a (non-SSA) variable. (Internally,
`FunctionBuilder` constructs SSA form to satisfy all uses).

`use_var`'s companion is `def_var`, used to write the value of a (non-SSA)
variable, which we use to implement assignment:

```rust
fn translate_assign(&mut self, name: String, expr: Expr) -> Value {
    // `def_var` is used to write the value of a variable. Note that
    // variables can have multiple definitions. Cranelift will
    // convert them into SSA form for itself automatically.
    let new_value = self.translate_expr(*expr);
    let variable = self.variables.get(&name).unwrap();
    self.builder.def_var(*variable, new_value);
    new_value
}
```

Next, let's dive into [if-else](./src/jit.rs#L243) expressions. In order to
demonstrate explicit SSA construction, this demo gives if-else expressions
return values. The way this works in Cranelift is that both arms of an
[if-else](./src/jit.rs#L268) have branches to a common merge point and pass
their "return values" as block parameters to the merge point.

Note that we seal each block once we know it has no more predecessors -
something that's straightforward to determine with a typical AST.

Putting it all together, here's the Cranelift IR for function
[foo](./examples/foo.txt), which contains multiple if-else expressions:

```
function u0:0(i64, i64) -> i64 system_v {
block0(v0: i64, v1: i64):
    v2 = iconst.i64 0
    brz v0, block2
    jump block1

block1:
    v4 = iconst.i64 0
    brz.i64 v1, block5
    jump block4

block4:
    v6 = iconst.i64 0
    v7 = iconst.i64 30
    jump block6(v7)

block5:
    v8 = iconst.i64 0
    v9 = iconst.i64 40
    jump block6(v9)

block6(v5: i64):
    jump block3(v5)

block2:
    v10 = iconst.i64 0
    v11 = iconst.i64 50
    jump block3(v11)

block3(v3: i64):
    v12 = iconst.i64 2
    v13 = iadd v3, v12
    return v13
}
```

The [while loop](./src/jit.rs#L325) translation is also straightforward. Here's
the Cranelift IR for function [sum](./examples/sum.txt), which contains a while
loop:

```
function u0:0(i64) -> i64 system_v {
block0(v0: i64):
    v1 = iconst.i64 0
    v2 = iconst.i64 0
    v3 = iconst.i64 1
    jump block1(v3, v2)  ; v3 = 1, v2 = 0

block1(v4: i64, v7: i64):
    v6 = icmp sle v4, v0
    brif v6, block2, block3

block2:
    v8 = iadd.i64 v7, v4
    v9 = iconst.i64 1
    v10 = iadd.i64 v4, v9  ; v9 = 1
    jump block1(v10, v8)

block3:
    v11 = iconst.i64 0
    return v7
}
```

For [function calls](./src/jit.rs#L357), the basic steps are to determine the
function call signature, declare the function to be called, put the function
arguments in an array, and invoke the `call` instruction.

The translation for [global data symbols](./src/jit.rs#L383) is similar: first,
declare the symbol to the module, then declare the symbol to the current
function, and finally use the `symbol_value` instruction to produce the value.

There's a ["Hello World" example](./src/bin/toy.rs#L63), which demonstrates
several other features. This example has to allocate some data to hold a
string. Using [`create_data`](./src/jit.rs#L97), we initialize a
`DataDescription` with the contents of the `hello_world` string and declare a
data object. We then use the `DataDescription` to define the object. At that
point, we're done with the `DataDescription` and can clear it. We call
`finalize_data` to perform linking and obtain the final runtime address of the
data, which we convert back into a Rust slice for convenience.

And to show off a handy feature of the JIT backend, we can look up symbols with
`libc::dlsym` and call libc functions such as `puts` (being careful to
NUL-terminate our strings). Unfortunately, calling `printf` requires varargs,
which Cranelift does not support yet.

And with all that, we can say "Hello World!"

### Native object files

Because of the `Module` abstraction, this demo can be adapted to write out an ELF
object file rather than JITing the code to memory with only minor changes, and I've done
so in [this branch](https://github.com/bytecodealliance/simplejit-demo/tree/faerie).
This writes a `test.o` file, which on an x86-64 ELF platform can be linked with
`cc test.o` to produce an executable that calls the generated functions,
including printing "Hello World!"

[Another branch](https://github.com/bytecodealliance/simplejit-demo/tree/faerie-macho)
shows how to write Mach-O object files.

Object files are written using the [faerie](https://github.com/m4b/faerie)
library.

### Have fun!

Cranelift is still evolving, so if there are things here which are confusing or
awkward, please let us know, via [GitHub
issues](https://github.com/bytecodealliance/wasmtime/issues?q=is%3Aissue+is%3Aopen+label%3Acranelift),
or just stop by the [Gitter chat](https://gitter.im/CraneStation/Lobby/~chat).
Very few things in Cranelift's design are set in stone at this time, and we're
really interested to hear from people about what makes sense and what doesn't.
