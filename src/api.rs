//! Module that defines the extern API of `Engine`.

use crate::any::{Dynamic, Variant};
use crate::engine::{
    get_script_function_by_signature, make_getter, make_setter, Engine, State, FUNC_INDEXER_GET,
    FUNC_INDEXER_SET,
};
use crate::error::ParseError;
use crate::fn_call::FuncArgs;
use crate::fn_native::{IteratorFn, SendSync};
use crate::fn_register::RegisterFn;
use crate::optimize::{optimize_into_ast, OptimizationLevel};
use crate::parser::AST;
use crate::result::EvalAltResult;
use crate::scope::Scope;
use crate::token::{lex, Position};
use crate::utils::StaticVec;

#[cfg(not(feature = "no_object"))]
use crate::engine::Map;

use crate::stdlib::{
    any::{type_name, TypeId},
    boxed::Box,
    mem,
    string::{String, ToString},
};

#[cfg(not(feature = "no_std"))]
#[cfg(not(target_arch = "wasm32"))]
use crate::stdlib::{fs::File, io::prelude::*, path::PathBuf};

/// Engine public API
impl Engine {
    /// Register a custom type for use with the `Engine`.
    /// The type must implement `Clone`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Debug, Clone, Eq, PartialEq)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                    { TestStruct { field: 1 } }
    ///     fn update(&mut self, offset: i64)   { self.field += offset; }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Use `register_fn` to register methods on the type.
    /// engine.register_fn("update", TestStruct::update);
    ///
    /// assert_eq!(
    ///     engine.eval::<TestStruct>("let x = new_ts(); x.update(41); x")?,
    ///     TestStruct { field: 42 }
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_type<T: Variant + Clone>(&mut self) {
        self.register_type_with_name::<T>(type_name::<T>());
    }

    /// Register a custom type for use with the `Engine`, with a pretty-print name
    /// for the `type_of` function. The type must implement `Clone`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self { TestStruct { field: 1 } }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// assert_eq!(
    ///     engine.eval::<String>("let x = new_ts(); type_of(x)")?,
    ///     "rust_out::TestStruct"
    /// );
    ///
    /// // Register the custom type with a name.
    /// engine.register_type_with_name::<TestStruct>("Hello");
    ///
    /// // Register methods on the type.
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// assert_eq!(
    ///     engine.eval::<String>("let x = new_ts(); type_of(x)")?,
    ///     "Hello"
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_type_with_name<T: Variant + Clone>(&mut self, name: &str) {
        // Add the pretty-print type name into the map
        self.type_names
            .insert(type_name::<T>().to_string(), name.to_string());
    }

    /// Register an iterator adapter for a type with the `Engine`.
    /// This is an advanced feature.
    pub fn register_iterator<T: Variant + Clone>(&mut self, f: IteratorFn) {
        self.global_module.set_iter(TypeId::of::<T>(), f);
    }

    /// Register a getter function for a member of a registered type with the `Engine`.
    ///
    /// The function signature must start with `&mut self` and not `&self`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                { TestStruct { field: 1 } }
    ///
    ///     // Even a getter must start with `&mut self` and not `&self`.
    ///     fn get_field(&mut self) -> i64  { self.field }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register a getter on a property (notice it doesn't have to be the same name).
    /// engine.register_get("xyz", TestStruct::get_field);
    ///
    /// assert_eq!(engine.eval::<i64>("let a = new_ts(); a.xyz")?, 1);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_get<T, U>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut T) -> U + SendSync + 'static,
    ) where
        T: Variant + Clone,
        U: Variant + Clone,
    {
        self.register_fn(&make_getter(name), callback);
    }

    /// Register a setter function for a member of a registered type with the `Engine`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Debug, Clone, Eq, PartialEq)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                        { TestStruct { field: 1 } }
    ///     fn set_field(&mut self, new_val: i64)   { self.field = new_val; }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register a setter on a property (notice it doesn't have to be the same name)
    /// engine.register_set("xyz", TestStruct::set_field);
    ///
    /// // Notice that, with a getter, there is no way to get the property value
    /// assert_eq!(
    ///     engine.eval::<TestStruct>("let a = new_ts(); a.xyz = 42; a")?,
    ///     TestStruct { field: 42 }
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_set<T, U>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut T, U) + SendSync + 'static,
    ) where
        T: Variant + Clone,
        U: Variant + Clone,
    {
        self.register_fn(&make_setter(name), callback);
    }

    /// Shorthand for registering both getter and setter functions
    /// of a registered type with the `Engine`.
    ///
    /// All function signatures must start with `&mut self` and not `&self`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                        { TestStruct { field: 1 } }
    ///     fn get_field(&mut self) -> i64          { self.field }
    ///     // Even a getter must start with `&mut self` and not `&self`.
    ///     fn set_field(&mut self, new_val: i64)   { self.field = new_val; }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register a getter and a setter on a property
    /// // (notice it doesn't have to be the same name)
    /// engine.register_get_set("xyz", TestStruct::get_field, TestStruct::set_field);
    ///
    /// assert_eq!(engine.eval::<i64>("let a = new_ts(); a.xyz = 42; a.xyz")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_get_set<T, U>(
        &mut self,
        name: &str,
        get_fn: impl Fn(&mut T) -> U + SendSync + 'static,
        set_fn: impl Fn(&mut T, U) + SendSync + 'static,
    ) where
        T: Variant + Clone,
        U: Variant + Clone,
    {
        self.register_get(name, get_fn);
        self.register_set(name, set_fn);
    }

    /// Register an index getter for a registered type with the `Engine`.
    ///
    /// The function signature must start with `&mut self` and not `&self`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     fields: Vec<i64>
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                { TestStruct { fields: vec![1, 2, 3, 4, 5] } }
    ///
    ///     // Even a getter must start with `&mut self` and not `&self`.
    ///     fn get_field(&mut self, index: i64) -> i64 { self.fields[index as usize] }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register an indexer.
    /// engine.register_indexer_get(TestStruct::get_field);
    ///
    /// assert_eq!(engine.eval::<i64>("let a = new_ts(); a[2]")?, 3);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    #[cfg(not(feature = "no_index"))]
    pub fn register_indexer_get<T, X, U>(
        &mut self,
        callback: impl Fn(&mut T, X) -> U + SendSync + 'static,
    ) where
        T: Variant + Clone,
        U: Variant + Clone,
        X: Variant + Clone,
    {
        self.register_fn(FUNC_INDEXER_GET, callback);
    }

    /// Register an index setter for a registered type with the `Engine`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     fields: Vec<i64>
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                { TestStruct { fields: vec![1, 2, 3, 4, 5] } }
    ///     fn set_field(&mut self, index: i64, value: i64) { self.fields[index as usize] = value; }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register an indexer.
    /// engine.register_indexer_set(TestStruct::set_field);
    ///
    /// assert_eq!(
    ///     engine.eval::<TestStruct>("let a = new_ts(); a[2] = 42; a")?.fields[2],
    ///     42
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    #[cfg(not(feature = "no_index"))]
    pub fn register_indexer_set<T, X, U>(
        &mut self,
        callback: impl Fn(&mut T, X, U) -> () + SendSync + 'static,
    ) where
        T: Variant + Clone,
        U: Variant + Clone,
        X: Variant + Clone,
    {
        self.register_fn(FUNC_INDEXER_SET, callback);
    }

    /// Shorthand for register both index getter and setter functions for a registered type with the `Engine`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     fields: Vec<i64>
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                { TestStruct { fields: vec![1, 2, 3, 4, 5] } }
    ///     fn get_field(&mut self, index: i64) -> i64      { self.fields[index as usize] }
    ///     fn set_field(&mut self, index: i64, value: i64) { self.fields[index as usize] = value; }
    /// }
    ///
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register an indexer.
    /// engine.register_indexer_get_set(TestStruct::get_field, TestStruct::set_field);
    ///
    /// assert_eq!(engine.eval::<i64>("let a = new_ts(); a[2] = 42; a[2]")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    #[cfg(not(feature = "no_index"))]
    pub fn register_indexer_get_set<T, X, U>(
        &mut self,
        getter: impl Fn(&mut T, X) -> U + SendSync + 'static,
        setter: impl Fn(&mut T, X, U) -> () + SendSync + 'static,
    ) where
        T: Variant + Clone,
        U: Variant + Clone,
        X: Variant + Clone,
    {
        self.register_indexer_get(getter);
        self.register_indexer_set(setter);
    }

    /// Compile a string into an `AST`, which can be used later for evaluation.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("40 + 2")?;
    ///
    /// for _ in 0..42 {
    ///     assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile(&self, script: &str) -> Result<AST, ParseError> {
        self.compile_with_scope(&Scope::new(), script)
    }

    /// Compile a string into an `AST` using own scope, which can be used later for evaluation.
    ///
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants
    /// // into function calls and operators.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 42_i64);   // 'x' is a constant
    ///
    /// // Compile a script to an AST and store it for later evaluation.
    /// // Notice that `Full` optimization is on, so constants are folded
    /// // into function calls and operators.
    /// let ast = engine.compile_with_scope(&mut scope,
    ///             "if x > 40 { x } else { 0 }"    // all 'x' are replaced with 42
    /// )?;
    ///
    /// // Normally this would have failed because no scope is passed into the 'eval_ast'
    /// // call and so the variable 'x' does not exist.  Here, it passes because the script
    /// // has been optimized and all references to 'x' are already gone.
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_with_scope(&self, scope: &Scope, script: &str) -> Result<AST, ParseError> {
        self.compile_scripts_with_scope(scope, &[script])
    }

    /// When passed a list of strings, first join the strings into one large script,
    /// and then compile them into an `AST` using own scope, which can be used later for evaluation.
    ///
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// ## Note
    ///
    /// All strings are simply parsed one after another with nothing inserted in between, not even
    /// a newline or space.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants
    /// // into function calls and operators.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 42_i64);   // 'x' is a constant
    ///
    /// // Compile a script made up of script segments to an AST and store it for later evaluation.
    /// // Notice that `Full` optimization is on, so constants are folded
    /// // into function calls and operators.
    /// let ast = engine.compile_scripts_with_scope(&mut scope, &[
    ///             "if x > 40",            // all 'x' are replaced with 42
    ///             "{ x } el",
    ///             "se { 0 }"              // segments do not need to be valid scripts!
    /// ])?;
    ///
    /// // Normally this would have failed because no scope is passed into the 'eval_ast'
    /// // call and so the variable 'x' does not exist.  Here, it passes because the script
    /// // has been optimized and all references to 'x' are already gone.
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_scripts_with_scope(
        &self,
        scope: &Scope,
        scripts: &[&str],
    ) -> Result<AST, ParseError> {
        self.compile_with_scope_and_optimization_level(scope, scripts, self.optimization_level)
    }

    /// Join a list of strings and compile into an `AST` using own scope at a specific optimization level.
    pub(crate) fn compile_with_scope_and_optimization_level(
        &self,
        scope: &Scope,
        scripts: &[&str],
        optimization_level: OptimizationLevel,
    ) -> Result<AST, ParseError> {
        let stream = lex(scripts, self.max_string_size);
        self.parse(&mut stream.peekable(), scope, optimization_level)
    }

    /// Read the contents of a file into a string.
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    fn read_file(path: PathBuf) -> Result<String, Box<EvalAltResult>> {
        let mut f = File::open(path.clone()).map_err(|err| {
            Box::new(EvalAltResult::ErrorReadingScriptFile(
                path.clone(),
                Position::none(),
                err,
            ))
        })?;

        let mut contents = String::new();

        f.read_to_string(&mut contents).map_err(|err| {
            Box::new(EvalAltResult::ErrorReadingScriptFile(
                path.clone(),
                Position::none(),
                err,
            ))
        })?;

        Ok(contents)
    }

    /// Compile a script file into an `AST`, which can be used later for evaluation.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script file to an AST and store it for later evaluation.
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let ast = engine.compile_file("script.rhai".into())?;
    ///
    /// for _ in 0..42 {
    ///     engine.eval_ast::<i64>(&ast)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn compile_file(&self, path: PathBuf) -> Result<AST, Box<EvalAltResult>> {
        self.compile_file_with_scope(&Scope::new(), path)
    }

    /// Compile a script file into an `AST` using own scope, which can be used later for evaluation.
    ///
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 42_i64);   // 'x' is a constant
    ///
    /// // Compile a script to an AST and store it for later evaluation.
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let ast = engine.compile_file_with_scope(&mut scope, "script.rhai".into())?;
    ///
    /// let result = engine.eval_ast::<i64>(&ast)?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn compile_file_with_scope(
        &self,
        scope: &Scope,
        path: PathBuf,
    ) -> Result<AST, Box<EvalAltResult>> {
        Self::read_file(path).and_then(|contents| Ok(self.compile_with_scope(scope, &contents)?))
    }

    /// Parse a JSON string into a map.
    ///
    /// Set `has_null` to `true` in order to map `null` values to `()`.
    /// Setting it to `false` will cause a _variable not found_ error during parsing.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// let map = engine.parse_json(r#"{"a":123, "b":42, "c":false, "d":null}"#, true)?;
    ///
    /// assert_eq!(map.len(), 4);
    /// assert_eq!(map.get("a").cloned().unwrap().cast::<i64>(), 123);
    /// assert_eq!(map.get("b").cloned().unwrap().cast::<i64>(), 42);
    /// assert_eq!(map.get("c").cloned().unwrap().cast::<bool>(), false);
    /// assert_eq!(map.get("d").cloned().unwrap().cast::<()>(), ());
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn parse_json(&self, json: &str, has_null: bool) -> Result<Map, Box<EvalAltResult>> {
        let mut scope = Scope::new();

        // Trims the JSON string and add a '#' in front
        let scripts = ["#", json.trim()];
        let stream = lex(&scripts, self.max_string_size);
        let ast =
            self.parse_global_expr(&mut stream.peekable(), &scope, OptimizationLevel::None)?;

        // Handle null - map to ()
        if has_null {
            scope.push_constant("null", ());
        }

        self.eval_ast_with_scope(&mut scope, &ast)
    }

    /// Compile a string containing an expression into an `AST`,
    /// which can be used later for evaluation.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile_expression("40 + 2")?;
    ///
    /// for _ in 0..42 {
    ///     assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_expression(&self, script: &str) -> Result<AST, ParseError> {
        self.compile_expression_with_scope(&Scope::new(), script)
    }

    /// Compile a string containing an expression into an `AST` using own scope,
    /// which can be used later for evaluation.
    ///
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants
    /// // into function calls and operators.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 10_i64);   // 'x' is a constant
    ///
    /// // Compile a script to an AST and store it for later evaluation.
    /// // Notice that `Full` optimization is on, so constants are folded
    /// // into function calls and operators.
    /// let ast = engine.compile_expression_with_scope(&mut scope,
    ///             "2 + (x + x) * 2"    // all 'x' are replaced with 10
    /// )?;
    ///
    /// // Normally this would have failed because no scope is passed into the 'eval_ast'
    /// // call and so the variable 'x' does not exist.  Here, it passes because the script
    /// // has been optimized and all references to 'x' are already gone.
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_expression_with_scope(
        &self,
        scope: &Scope,
        script: &str,
    ) -> Result<AST, ParseError> {
        let scripts = [script];
        let stream = lex(&scripts, self.max_string_size);
        {
            let mut peekable = stream.peekable();
            self.parse_global_expr(&mut peekable, scope, self.optimization_level)
        }
    }

    /// Evaluate a script file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let result = engine.eval_file::<i64>("script.rhai".into())?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn eval_file<T: Variant + Clone>(&self, path: PathBuf) -> Result<T, Box<EvalAltResult>> {
        Self::read_file(path).and_then(|contents| self.eval::<T>(&contents))
    }

    /// Evaluate a script file with own scope.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 42_i64);
    ///
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let result = engine.eval_file_with_scope::<i64>(&mut scope, "script.rhai".into())?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn eval_file_with_scope<T: Variant + Clone>(
        &self,
        scope: &mut Scope,
        path: PathBuf,
    ) -> Result<T, Box<EvalAltResult>> {
        Self::read_file(path).and_then(|contents| self.eval_with_scope::<T>(scope, &contents))
    }

    /// Evaluate a string.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// assert_eq!(engine.eval::<i64>("40 + 2")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval<T: Variant + Clone>(&self, script: &str) -> Result<T, Box<EvalAltResult>> {
        self.eval_with_scope(&mut Scope::new(), script)
    }

    /// Evaluate a string with own scope.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 40_i64);
    ///
    /// assert_eq!(engine.eval_with_scope::<i64>(&mut scope, "x = x + 2; x")?, 42);
    /// assert_eq!(engine.eval_with_scope::<i64>(&mut scope, "x = x + 2; x")?, 44);
    ///
    /// // The variable in the scope is modified
    /// assert_eq!(scope.get_value::<i64>("x").expect("variable x should exist"), 44);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_with_scope<T: Variant + Clone>(
        &self,
        scope: &mut Scope,
        script: &str,
    ) -> Result<T, Box<EvalAltResult>> {
        let ast = self.compile_with_scope_and_optimization_level(
            scope,
            &[script],
            self.optimization_level,
        )?;
        self.eval_ast_with_scope(scope, &ast)
    }

    /// Evaluate a string containing an expression.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// assert_eq!(engine.eval_expression::<i64>("40 + 2")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_expression<T: Variant + Clone>(
        &self,
        script: &str,
    ) -> Result<T, Box<EvalAltResult>> {
        self.eval_expression_with_scope(&mut Scope::new(), script)
    }

    /// Evaluate a string containing an expression with own scope.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 40_i64);
    ///
    /// assert_eq!(engine.eval_expression_with_scope::<i64>(&mut scope, "x + 2")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_expression_with_scope<T: Variant + Clone>(
        &self,
        scope: &mut Scope,
        script: &str,
    ) -> Result<T, Box<EvalAltResult>> {
        let scripts = [script];
        let stream = lex(&scripts, self.max_string_size);

        // No need to optimize a lone expression
        let ast = self.parse_global_expr(&mut stream.peekable(), scope, OptimizationLevel::None)?;

        self.eval_ast_with_scope(scope, &ast)
    }

    /// Evaluate an `AST`.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("40 + 2")?;
    ///
    /// // Evaluate it
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_ast<T: Variant + Clone>(&self, ast: &AST) -> Result<T, Box<EvalAltResult>> {
        self.eval_ast_with_scope(&mut Scope::new(), ast)
    }

    /// Evaluate an `AST` with own scope.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("x + 2")?;
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 40_i64);
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("x = x + 2; x")?;
    ///
    /// // Evaluate it
    /// assert_eq!(engine.eval_ast_with_scope::<i64>(&mut scope, &ast)?, 42);
    /// assert_eq!(engine.eval_ast_with_scope::<i64>(&mut scope, &ast)?, 44);
    ///
    /// // The variable in the scope is modified
    /// assert_eq!(scope.get_value::<i64>("x").expect("variable x should exist"), 44);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_ast_with_scope<T: Variant + Clone>(
        &self,
        scope: &mut Scope,
        ast: &AST,
    ) -> Result<T, Box<EvalAltResult>> {
        let (result, _) = self.eval_ast_with_scope_raw(scope, ast)?;

        let return_type = self.map_type_name(result.type_name());

        return result.try_cast::<T>().ok_or_else(|| {
            Box::new(EvalAltResult::ErrorMismatchOutputType(
                return_type.into(),
                Position::none(),
            ))
        });
    }

    /// Evaluate an `AST` with own scope.
    pub(crate) fn eval_ast_with_scope_raw(
        &self,
        scope: &mut Scope,
        ast: &AST,
    ) -> Result<(Dynamic, u64), Box<EvalAltResult>> {
        let mut state = State::new();

        ast.statements()
            .iter()
            .try_fold(().into(), |_, stmt| {
                self.eval_stmt(scope, &mut state, ast.lib(), stmt, 0)
            })
            .or_else(|err| match *err {
                EvalAltResult::Return(out, _) => Ok(out),
                _ => Err(err),
            })
            .map(|v| (v, state.operations))
    }

    /// Evaluate a file, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn consume_file(&self, path: PathBuf) -> Result<(), Box<EvalAltResult>> {
        Self::read_file(path).and_then(|contents| self.consume(&contents))
    }

    /// Evaluate a file with own scope, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    #[cfg(not(feature = "no_std"))]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn consume_file_with_scope(
        &self,
        scope: &mut Scope,
        path: PathBuf,
    ) -> Result<(), Box<EvalAltResult>> {
        Self::read_file(path).and_then(|contents| self.consume_with_scope(scope, &contents))
    }

    /// Evaluate a string, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume(&self, script: &str) -> Result<(), Box<EvalAltResult>> {
        self.consume_with_scope(&mut Scope::new(), script)
    }

    /// Evaluate a string with own scope, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume_with_scope(
        &self,
        scope: &mut Scope,
        script: &str,
    ) -> Result<(), Box<EvalAltResult>> {
        let scripts = [script];
        let stream = lex(&scripts, self.max_string_size);
        let ast = self.parse(&mut stream.peekable(), scope, self.optimization_level)?;
        self.consume_ast_with_scope(scope, &ast)
    }

    /// Evaluate an AST, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume_ast(&self, ast: &AST) -> Result<(), Box<EvalAltResult>> {
        self.consume_ast_with_scope(&mut Scope::new(), ast)
    }

    /// Evaluate an `AST` with own scope, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume_ast_with_scope(
        &self,
        scope: &mut Scope,
        ast: &AST,
    ) -> Result<(), Box<EvalAltResult>> {
        let mut state = State::new();

        ast.statements()
            .iter()
            .try_fold(().into(), |_, stmt| {
                self.eval_stmt(scope, &mut state, ast.lib(), stmt, 0)
            })
            .map_or_else(
                |err| match *err {
                    EvalAltResult::Return(_, _) => Ok(()),
                    err => Err(Box::new(err)),
                },
                |_| Ok(()),
            )
    }

    /// Call a script function defined in an `AST` with multiple arguments.
    /// Arguments are passed as a tuple.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_function"))]
    /// # {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// let ast = engine.compile(r"
    ///     fn add(x, y) { len(x) + y + foo }
    ///     fn add1(x)   { len(x) + 1 + foo }
    ///     fn bar()     { foo/2 }
    /// ")?;
    ///
    /// let mut scope = Scope::new();
    /// scope.push("foo", 42_i64);
    ///
    /// // Call the script-defined function
    /// let result: i64 = engine.call_fn(&mut scope, &ast, "add", ( String::from("abc"), 123_i64 ) )?;
    /// assert_eq!(result, 168);
    ///
    /// let result: i64 = engine.call_fn(&mut scope, &ast, "add1", ( String::from("abc"), ) )?;
    /// //                                                         ^^^^^^^^^^^^^^^^^^^^^^^^ tuple of one
    /// assert_eq!(result, 46);
    ///
    /// let result: i64 = engine.call_fn(&mut scope, &ast, "bar", () )?;
    /// assert_eq!(result, 21);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_function"))]
    pub fn call_fn<A: FuncArgs, T: Variant + Clone>(
        &self,
        scope: &mut Scope,
        ast: &AST,
        name: &str,
        args: A,
    ) -> Result<T, Box<EvalAltResult>> {
        let mut arg_values = args.into_vec();
        let result = self.call_fn_dynamic_raw(scope, ast, name, arg_values.as_mut())?;

        let return_type = self.map_type_name(result.type_name());

        return result.try_cast().ok_or_else(|| {
            Box::new(EvalAltResult::ErrorMismatchOutputType(
                return_type.into(),
                Position::none(),
            ))
        });
    }

    /// Call a script function defined in an `AST` with multiple `Dynamic` arguments.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # #[cfg(not(feature = "no_function"))]
    /// # {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// let ast = engine.compile(r"
    ///     fn add(x, y) { len(x) + y + foo }
    ///     fn add1(x)   { len(x) + 1 + foo }
    ///     fn bar()     { foo/2 }
    /// ")?;
    ///
    /// let mut scope = Scope::new();
    /// scope.push("foo", 42_i64);
    ///
    /// // Call the script-defined function
    /// let result = engine.call_fn_dynamic(&mut scope, &ast, "add", vec![ String::from("abc").into(), 123_i64.into() ])?;
    /// assert_eq!(result.cast::<i64>(), 168);
    ///
    /// let result = engine.call_fn_dynamic(&mut scope, &ast, "add1", vec![ String::from("abc").into() ])?;
    /// assert_eq!(result.cast::<i64>(), 46);
    ///
    /// let result= engine.call_fn_dynamic(&mut scope, &ast, "bar", vec![])?;
    /// assert_eq!(result.cast::<i64>(), 21);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_function"))]
    pub fn call_fn_dynamic(
        &self,
        scope: &mut Scope,
        ast: &AST,
        name: &str,
        arg_values: impl IntoIterator<Item = Dynamic>,
    ) -> Result<Dynamic, Box<EvalAltResult>> {
        let mut arg_values: StaticVec<_> = arg_values.into_iter().collect();
        self.call_fn_dynamic_raw(scope, ast, name, arg_values.as_mut())
    }

    /// Call a script function defined in an `AST` with multiple `Dynamic` arguments.
    ///
    /// ## WARNING
    ///
    /// All the arguments are _consumed_, meaning that they're replaced by `()`.
    /// This is to avoid unnecessarily cloning the arguments.
    /// Do not use the arguments after this call. If they are needed afterwards,
    /// clone them _before_ calling this function.
    pub(crate) fn call_fn_dynamic_raw(
        &self,
        scope: &mut Scope,
        ast: &AST,
        name: &str,
        arg_values: &mut [Dynamic],
    ) -> Result<Dynamic, Box<EvalAltResult>> {
        let mut args: StaticVec<_> = arg_values.iter_mut().collect();
        let fn_def = get_script_function_by_signature(ast.lib(), name, args.len(), true)
            .ok_or_else(|| {
                Box::new(EvalAltResult::ErrorFunctionNotFound(
                    name.into(),
                    Position::none(),
                ))
            })?;

        let mut state = State::new();
        let args = args.as_mut();

        self.call_script_fn(scope, &mut state, ast.lib(), name, fn_def, args, 0)
    }

    /// Optimize the `AST` with constants defined in an external Scope.
    /// An optimized copy of the `AST` is returned while the original `AST` is consumed.
    ///
    /// Although optimization is performed by default during compilation, sometimes it is necessary to
    /// _re_-optimize an AST. For example, when working with constants that are passed in via an
    /// external scope, it will be more efficient to optimize the `AST` once again to take advantage
    /// of the new constants.
    ///
    /// With this method, it is no longer necessary to recompile a large script. The script `AST` can be
    /// compiled just once. Before evaluation, constants are passed into the `Engine` via an external scope
    /// (i.e. with `scope.push_constant(...)`). Then, the `AST is cloned and the copy re-optimized before running.
    #[cfg(not(feature = "no_optimize"))]
    pub fn optimize_ast(
        &self,
        scope: &Scope,
        mut ast: AST,
        optimization_level: OptimizationLevel,
    ) -> AST {
        let lib = ast
            .lib()
            .iter_fn()
            .filter(|(_, _, _, f)| f.is_script())
            .map(|(_, _, _, f)| f.get_fn_def().clone())
            .collect();

        let stmt = mem::take(ast.statements_mut());
        optimize_into_ast(self, scope, stmt, lib, optimization_level)
    }

    /// Register a callback for script evaluation progress.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # use std::sync::RwLock;
    /// # use std::sync::Arc;
    /// use rhai::Engine;
    ///
    /// let result = Arc::new(RwLock::new(0_u64));
    /// let logger = result.clone();
    ///
    /// let mut engine = Engine::new();
    ///
    /// engine.on_progress(move |&ops| {
    ///     if ops > 10000 {
    ///         false
    ///     } else if ops % 800 == 0 {
    ///         *logger.write().unwrap() = ops;
    ///         true
    ///     } else {
    ///         true
    ///     }
    /// });
    ///
    /// engine.consume("for x in range(0, 50000) {}")
    ///     .expect_err("should error");
    ///
    /// assert_eq!(*result.read().unwrap(), 9600);
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_progress(&mut self, callback: impl Fn(&u64) -> bool + SendSync + 'static) {
        self.progress = Some(Box::new(callback));
    }

    /// Override default action of `print` (print to stdout using `println!`)
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # use std::sync::RwLock;
    /// # use std::sync::Arc;
    /// use rhai::Engine;
    ///
    /// let result = Arc::new(RwLock::new(String::from("")));
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Override action of 'print' function
    /// let logger = result.clone();
    /// engine.on_print(move |s| logger.write().unwrap().push_str(s));
    ///
    /// engine.consume("print(40 + 2);")?;
    ///
    /// assert_eq!(*result.read().unwrap(), "42");
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_print(&mut self, callback: impl Fn(&str) + SendSync + 'static) {
        self.print = Box::new(callback);
    }

    /// Override default action of `debug` (print to stdout using `println!`)
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), Box<rhai::EvalAltResult>> {
    /// # use std::sync::RwLock;
    /// # use std::sync::Arc;
    /// use rhai::Engine;
    ///
    /// let result = Arc::new(RwLock::new(String::from("")));
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Override action of 'print' function
    /// let logger = result.clone();
    /// engine.on_debug(move |s| logger.write().unwrap().push_str(s));
    ///
    /// engine.consume(r#"debug("hello");"#)?;
    ///
    /// assert_eq!(*result.read().unwrap(), r#""hello""#);
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_debug(&mut self, callback: impl Fn(&str) + SendSync + 'static) {
        self.debug = Box::new(callback);
    }
}
