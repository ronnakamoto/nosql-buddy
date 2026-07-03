//! zk-audit-ceremony — generate Groth16 proving/verifying keys from an R1CS.
//!
//! This is the "Powers of Tau" step for the audit circuit. Run it once to
//! produce a stable proving key that can be reused for every proof.
//!
//! Usage:
//!   zk-audit-ceremony <r1cs_path> <output_dir>
//!
//! Outputs (named after the R1CS file stem, e.g. `merkle_inclusion.r1cs`
//! → `merkle_inclusion.pkey` / `.vkey`):
//!   <output_dir>/<stem>.pkey  — proving key (arkworks binary)
//!   <output_dir>/<stem>.vkey  — verifying key (arkworks binary)

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "zk-audit-ceremony — generate Groth16 proving/verifying keys\n\
            \n\
            Usage:\n\
              zk-audit-ceremony <r1cs_path> <output_dir>\n\
            \n\
            Outputs (named after the R1CS file stem):\n\
              <output_dir>/<stem>.pkey  — proving key\n\
              <output_dir>/<stem>.vkey  — verifying key"
        );
        std::process::exit(1);
    }

    let r1cs_path = &args[1];
    let output_dir = Path::new(&args[2]);

    if !Path::new(r1cs_path).exists() {
        eprintln!("error: R1CS file not found: {}", r1cs_path);
        std::process::exit(1);
    }

    std::fs::create_dir_all(output_dir).expect("failed to create output directory");

    let stem = Path::new(r1cs_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("merkle_inclusion");
    let pkey_path = output_dir.join(format!("{stem}.pkey"));
    let vkey_path = output_dir.join(format!("{stem}.vkey"));

    println!("zk-audit-ceremony: generating Groth16 parameters from {}", r1cs_path);
    println!("  proving key:  {}", pkey_path.display());
    println!("  verifying key: {}", vkey_path.display());

    zk_audit::generate_and_save_parameters(
        r1cs_path,
        pkey_path.to_str().unwrap(),
        vkey_path.to_str().unwrap(),
    )
    .expect("ceremony failed");

    println!("done.");
}
