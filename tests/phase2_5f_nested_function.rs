//! Integration test: Phase 2.5f — nested `local function`
//! definitions (ADR 0036). No upvalue capture; body cannot reach
//! into the outer function's scope.

use std::process::Command;

fn run(src: &str, output_name: &str) -> String {
    let output = std::env::temp_dir().join(output_name);
    let chunk = lumelir::parser::parse(src).unwrap();
    let hir = lumelir::hir::lower(&chunk).unwrap();
    lumelir::codegen::compile(&hir, &output).unwrap();
    let result = Command::new(&output)
        .output()
        .expect("failed to run compiled binary");
    assert!(result.status.success(), "binary should exit 0");
    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
    let _ = std::fs::remove_file(&output);
    stdout
}

#[test]
fn nested_function_called_from_outer() {
    let src = "local function outer()
  local function inner()
    return 7
  end
  return inner()
end
print(outer())";
    assert_eq!(run(src, "lumelir_25f_inner").trim(), "7");
}

#[test]
fn nested_function_recursive_self_reference() {
    let src = "local function outer(x)
  local function fact(n)
    if n == 0 then return 1 end
    return n * fact(n - 1)
  end
  return fact(x)
end
print(outer(5))";
    assert_eq!(run(src, "lumelir_25f_fact").trim(), "120");
}

#[test]
fn two_sibling_nested_functions() {
    let src = "local function outer()
  local function double(n) return n * 2 end
  local function inc(n) return n + 1 end
  return double(inc(3))
end
print(outer())";
    assert_eq!(run(src, "lumelir_25f_siblings").trim(), "8");
}

#[test]
fn nested_function_called_after_call() {
    // Forward-reference between siblings: `g` calls `h`, declared
    // later. `g`'s body resolves `h` via function_names registered
    // at HIR pass-1 of the outer function; the nested case mirrors
    // that mechanic at lower_stmt time.
    let src = "local function outer()
  local function g(n) return h(n) + 1 end
  local function h(n) return n * 10 end
  return g(5)
end
print(outer())";
    // g(5) -> h(5)+1 -> 50+1 = 51
    assert_eq!(run(src, "lumelir_25f_forward").trim(), "51");
}

#[test]
fn nested_function_referencing_outer_param_is_static_error() {
    // Phase 2.5c (closures) will lift this restriction. For now
    // we want a clean static error rather than miscompilation.
    let chunk = lumelir::parser::parse(
        "local function outer(x)
  local function inner()
    return x
  end
  return inner()
end",
    )
    .unwrap();
    assert!(lumelir::hir::lower(&chunk).is_err());
}

#[test]
fn deeply_nested_three_levels() {
    let src = "local function a()
  local function b()
    local function c()
      return 42
    end
    return c()
  end
  return b()
end
print(a())";
    assert_eq!(run(src, "lumelir_25f_deep").trim(), "42");
}
