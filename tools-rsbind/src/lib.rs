#![recursion_limit = "128"]
extern crate syn;
#[macro_use]
extern crate quote;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate cbindgen;
extern crate fs_extra;
extern crate proc_macro2;
extern crate serde;
extern crate toml;
extern crate zip;

mod android;
mod ast;
mod bridge;
mod build;
mod cargo;
mod config;
mod errors;
mod ios;
mod unzip;

use android::process::AndroidProcess;
use ast::AstResult;
use build::*;
use errors::*;
use ios::process::IosProcess;
use std::fs;
use std::path::PathBuf;

const GEN_DIR_NAME: &str = "_gen";
const HEADER_NAME: &str = "header";
const AST_DIR: &str = "ast";
const IOS_PROJ: &str = "ios_dest";
const IOS_BRIDGE_PROJ: &str = "ios_bridge";
const ANDROID_BRIDGE_PROJ: &str = "android_bridge";
const ANDROID_PROJ: &str = "android_dest";
const BIN_DIR: &str = "bin";

pub struct Bind {
    prj_path: PathBuf,
    ios_dest_path: PathBuf,
    ios_bridge_path: PathBuf,
    android_bridge_path: PathBuf,
    android_dest_path: PathBuf,
    header_path: PathBuf,
    ast_path: PathBuf,
    bin_path: PathBuf,
    target: Target,
    action: Action,
}

pub enum Target {
    Android,
    Ios,
    All,
}

pub enum Action {
    GEN_AST,
    GEN_BRIDGE,
    GEN_BIND_SRC,
    GEN_C_HEADER,
    BUILD,
    ALL,
}

impl Bind {
    ///
    /// crate the object for binding.
    /// * prject_path: the rust project we need to bind
    /// * target: which target we want to generate, android or iOS
    ///
    pub fn from(prj_path: String, target: Target, action: Action) -> Bind {
        let root = PathBuf::from(&prj_path);

        // ./_gen/ast
        let ast_path = root.join(GEN_DIR_NAME).join(AST_DIR);

        // ./_gen/bin/
        let bin_path = root.join(GEN_DIR_NAME).join(BIN_DIR);

        // ./_gen/header/
        let header_path = root.join(GEN_DIR_NAME).join(HEADER_NAME);

        // ./_gen/ios_dest/
        let ios_dest_path = root.join(GEN_DIR_NAME).join(IOS_PROJ);

        // ./_gen/ios_bridge
        let ios_bridge_path = root.join(GEN_DIR_NAME).join(IOS_BRIDGE_PROJ);

        let android_bridge_path = root.join(GEN_DIR_NAME).join(ANDROID_BRIDGE_PROJ);

        let android_dest_path = root.join(GEN_DIR_NAME).join(ANDROID_PROJ);

        return Bind {
            prj_path: root,
            ios_dest_path,
            ios_bridge_path,
            android_bridge_path,
            android_dest_path,
            header_path,
            ast_path,
            bin_path,
            target,
            action,
        };
    }

    ///
    /// generate the ios framework and android aar as per the target config
    ///
    pub fn gen_all(&self) -> Result<()> {
        let config = config::parse(&self.prj_path);
        println!("rsbind config in {:?} is {:?}", &self.prj_path, config);

        let crate_name = self.parse_crate_name()?;

        match self.action {
            Action::GEN_AST => {
                self.parse_ast(crate_name.clone()).unwrap();
                return Ok(());
            }
            _ => (),
        }

        Ok(match self.target {
            Target::Ios => {
                let ast = &self.get_ast_if_need(crate_name.clone()).unwrap();
                self.gen_for_ios(&crate_name, ast, config.clone()).unwrap();
            }
            Target::Android => {
                let ast = &self.get_ast_if_need(crate_name.clone()).unwrap();
                self.gen_for_android(&crate_name, ast, config.clone())
                    .unwrap();
            }
            Target::All => {
                let ast_result = self.get_ast_if_need(crate_name.clone()).unwrap();
                self.gen_for_ios(&crate_name, &ast_result, config.clone())
                    .unwrap();
                self.gen_for_android(&crate_name, &ast_result, config.clone())
                    .unwrap();
            }
        })
    }

    fn get_ast_if_need(&self, crate_name: String) -> Result<AstResult> {
        match self.action {
            Action::GEN_BRIDGE | Action::GEN_BIND_SRC | Action::ALL => {
                self.parse_ast(crate_name.clone())
            }
            _ => {
                use std::collections::HashMap;
                let ast_result = AstResult {
                    trait_descs: HashMap::new(),
                    struct_descs: HashMap::new(),
                    imp_desc: vec![],
                };
                Ok(ast_result)
            }
        }
    }

    fn parse_ast(&self, crate_name: String) -> Result<AstResult> {
        let prj_path = PathBuf::from(&self.prj_path);
        if self.ast_path.exists() {
            fs::remove_dir_all(&self.ast_path).unwrap();
        }
        fs::create_dir_all(&self.ast_path).unwrap();
        return ast::AstHandler::new(crate_name)
            .parse(&prj_path)
            .unwrap()
            .flush(&self.ast_path);
    }

    ///
    /// generate the ios framework
    fn gen_for_ios(
        &self,
        crate_name: &str,
        ast_result: &AstResult,
        config: Option<config::Config>,
    ) -> Result<()> {
        let ios_process = IosProcess::new(
            &self.prj_path,
            &self.ios_dest_path,
            &self.ios_bridge_path,
            &self.header_path,
            &self.ast_path,
            &self.bin_path,
            crate_name,
            &ast_result,
            config.clone(),
        );

        match self.action {
            Action::GEN_AST => (),
            Action::GEN_BRIDGE => ios_process.gen_bridge_src().unwrap(),
            Action::GEN_BIND_SRC => ios_process.gen_bind_code().unwrap(),
            Action::GEN_C_HEADER => ios_process.gen_c_header().unwrap(),
            Action::BUILD => {
                ios_process.build_bridge_prj().unwrap();
                ios_process.copy_bridge_outputs().unwrap();
                ios_process.build_dest_prj().unwrap();
            }
            Action::ALL => {
                ios_process.gen_bridge_src().unwrap();
                ios_process.gen_bind_code().unwrap();
                ios_process.build_bridge_prj().unwrap();
                ios_process.copy_bridge_outputs().unwrap();
                ios_process.build_dest_prj().unwrap();
            }
        }

        Ok(())
    }

    fn gen_c_header(
        &self,
        crate_name: &str,
        ast_result: &AstResult,
        config: &Option<config::Config>,
    ) -> Result<()> {
        let ios_process = IosProcess::new(
            &self.prj_path,
            &self.ios_dest_path,
            &self.ios_bridge_path,
            &self.header_path,
            &self.ast_path,
            &self.bin_path,
            crate_name,
            &ast_result,
            config.clone(),
        );

        ios_process.gen_c_header();

        Ok(())
    }

    ///
    /// generate the android aar
    ///
    fn gen_for_android(
        &self,
        crate_name: &str,
        ast_result: &AstResult,
        config: Option<config::Config>,
    ) -> Result<()> {
        let android_process = AndroidProcess::new(
            &self.prj_path,
            &self.android_dest_path,
            &self.android_bridge_path,
            &self.header_path,
            &self.ast_path,
            &self.bin_path,
            crate_name,
            ast_result,
            config,
        );

        match self.action {
            Action::GEN_AST => (),
            Action::GEN_BRIDGE => android_process.gen_bridge_src().unwrap(),
            Action::GEN_BIND_SRC => android_process.gen_bind_code().unwrap(),
            Action::GEN_C_HEADER => (),
            Action::BUILD => {
                android_process.build_bridge_prj().unwrap();
                android_process.copy_bridge_outputs().unwrap();
                android_process.build_dest_prj().unwrap();
            }
            Action::ALL => {
                android_process.gen_bridge_src().unwrap();
                android_process.gen_bind_code().unwrap();
                android_process.build_bridge_prj().unwrap();
                android_process.copy_bridge_outputs().unwrap();
                android_process.build_dest_prj().unwrap();
            }
        };

        Ok(())
    }

    ///
    /// parse the crate name of origin project from Cargo.toml
    ///
    fn parse_crate_name(&self) -> Result<String> {
        let toml_path = PathBuf::from(&self.prj_path).join("Cargo.toml");
        let manifest = cargo::manifest(toml_path.as_path())?;
        println!("parse project name = {}", manifest.package.name);
        Ok(manifest.package.name)
    }
}