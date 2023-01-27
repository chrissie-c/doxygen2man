// doxygen2man
//
// Copyright (C) 2020-2021 Red Hat, Inc.  All rights reserved.
//
// Author: Christine Caulfield <ccaulfie@redhat.com>
//
// This software licensed under GPL-2.0+
//

extern crate xml;
extern crate chrono;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write, ErrorKind, Error, BufRead};
use std::fmt::Write as fmtwrite;
use structopt::StructOpt;
use xml::reader::{EventReader, XmlEvent, ParserConfig};
use xml::name::OwnedName;
use chrono::prelude::*;

// This defines how long a parameter type can get before we
// decide it's not worth lining everything up.
// It's mainly to stop function pointer types (which can get VERY long because
// of all *their* parameters) making everything else 'line-up' over separate lines

const MAX_PRINT_PARAM_LEN: usize = 80;

// Similar for structure member comments
const MAX_STRUCT_COMMENT_LEN: usize = 50;


#[derive(Debug, StructOpt)]
#[structopt(name = "doxygen2man", about = "Convert doxygen files to man pages")]
/// This is a tool to generate API manpages from a doxygen-annotated header file.
/// First run doxygen on the file and then run this program against the main XML file
/// it created and the directory containing the ancilliary files. It will then
/// output a lot of *.3 man page files which you can then ship with your library.
///
/// Doxygen creates an .xml file for each .h file in your project, pass these .xml
/// files into this program - they are usually called something like
/// <include-file>_8h.xml, eg qbipcs_8h.xml, you can pass multiple XML files to
/// doxygen2man on the command-line.
///
/// If you want HTML output then simpy use nroff on the generated files as you
/// would do with any other man page.
///
struct Opt {
    #[structopt (short="a", long="print-ascii", help="Print ASCII dump of manpage data to stdout")]
    print_ascii: bool,

    #[structopt (short="m", long="print-man", help="Write man page files to <output-dir>")]
    print_man: bool,

    #[structopt (short="P", long="print-params", help="print PARAMS section")]
    print_params: bool,

    #[structopt (short="g", long="print-general", help="Print general man page for the whole header file")]
    print_general: bool,

    #[structopt (short="q", long="quiet", help="Run quietly, no progress info printed")]
    _quiet: bool,

    #[structopt (short="c", long="use-header-copyright", help="Use the Copyright date from the header file (if one can be found)")]
    use_header_copyright: bool,

    #[structopt (short="I", long="headerfile", default_value="unknown.h", help="Set include filename (default taken from XML)")]
    headerfile: String,

    #[structopt (short="i", long="header-prefix", default_value="", help="prefix for includefile. eg qb/")]
    header_prefix: String,

    #[structopt (short="s", long="section", default_value="3", help="write man pages into section <section>")]
    man_section: u32,

    #[structopt (short="S", long="start-year", default_value="2010", help="Start year to print at end of copyright line")]
    start_year: u32,

    #[structopt (short="d", long="xml-dir", default_value="./xml/", help="Directory for XML files")]
    xml_dir: String,

    #[structopt (short="D", long="manpage-date", default_value="2010", help="Date to print at top of man pages (format not checked)")]
    manpage_date: String,

    #[structopt (short="Y", long="manpage-year", default_value="2010", help="Year to print at end of copyright line")]
    manpage_year: i32,

    #[structopt (short="p", long="package-name", default_value="Package", help="Name of package for these man pages")]
    package_name: String,

    #[structopt (short="H", long="header-name", default_value="Programmer's Manual", help="Header text")]
    header: String,

    #[structopt (short="o", long="output_dir", default_value="./", help="Write all man pages to <dir>")]
    output_dir: String,

    #[structopt (short="O", long="header_src_dir", default_value="./", help="Directory for the original header files (often needed by -c above)")]
    header_src_dir: String,

    #[structopt (short="C", long="company", default_value="Red Hat Inc", help="Company name in copyright")]
    company: String,

    // Positional parameters
    #[structopt (help="XML files to process", required = true)]
    xml_files: Vec<String>,
}

// Function parameter - also used for structure members
#[derive(Clone)]
struct FnParam
{
    par_name: String,
    par_type: String,
    par_refid: Option<String>,
    par_args: String,
    par_desc: String,
    par_brief: String,
}

#[derive(Clone)]
struct ReturnVal
{
    ret_name: String,
    ret_desc: String,
}

#[derive(Clone)]
enum StructureType
{
    Unknown,
    Enum,
    Struct,
}
#[derive(Clone)]
struct StructureInfo
{
    str_type: StructureType,
    str_name: String,
    str_brief: String,
    str_description: String,
    str_members: Vec<FnParam>,
}

impl StructureInfo {
    pub fn new() -> StructureInfo {
        StructureInfo {
            str_type: StructureType::Unknown,
            str_name: String::new(),
            str_brief: String::new(),
            str_description: String::new(),
            str_members: Vec::<FnParam>::new(),
        }
    }
}

// Collected #defines - printed on the General page.
struct HashDefine
{
    hd_name: String,
    hd_init: String,
    hd_brief: String,
    hd_desc: String,
}


// Information for a function.
// Pretty much everything else is hung off this
struct FunctionInfo
{
    fn_type: String,
    fn_name: String,
    fn_def: String,
    fn_argsstring: String,
    fn_brief: String,
    fn_detail: String,
    fn_returnval: String,
    fn_note: String,
    fn_args: Vec<FnParam>,
    fn_defines: Vec<HashDefine>,
    fn_retvals: Vec<ReturnVal>,
    fn_refids: Vec<String>, // refids for structs used in the function
}

impl FunctionInfo {
    pub fn new() -> FunctionInfo {
        FunctionInfo {
            fn_type: String::new(),
            fn_name: String::new(),
            fn_def: String::new(),
            fn_argsstring: String::new(),
            fn_brief: String::new(),
            fn_detail: String::new(),
            fn_returnval: String::new(),
            fn_note: String::new(),
            fn_args: Vec::<FnParam>::new(),
            fn_defines: Vec::<HashDefine>::new(),
            fn_retvals: Vec::<ReturnVal>::new(),
            fn_refids: Vec::<String>::new(),
        }
    }
}

// Return the length of a string ignoring any formatting
fn len_without_formatting(param: &str) -> usize
{
    let mut length = 0;
    let mut last_was_escape = false;
    for i in param.chars() {
	if i == '\\' {
	    last_was_escape = true;
	} else if last_was_escape {
	    last_was_escape = false;
	} else {
	    length += 1;
	}
    }
    length
}

// Does what it says on the tin
fn get_attr(e: &XmlEvent, attrname: &str) -> String
{
    if let XmlEvent::StartElement {attributes,.. } = e {
        for a in attributes {
            if a.name.to_string() == attrname {
                return a.value.to_string();
            }
        }
    }
    String::new()
}


// Do the easy/common tags here
fn parse_standard_elements(parser: &mut EventReader<BufReader<File>>, name: &OwnedName, e: &XmlEvent) -> Result<String, xml::reader::Error>
{
    let mut text = String::new();

    match name.to_string().as_str() {
        "para" => {
            text += collect_text(parser, name)?.as_str();
        }
        "sp" => {
            text += " ";
        }
        "emphasis" => {
            text += "\\fB";
            text += collect_text(parser, name)?.as_str();
            text += "\\fR";
        }
        "highlight" => { // TBH I've only ever seen "normal" here
            let h_type = get_attr(e, "class");
            if h_type != "normal" {
                text += "\\fB";
            }
            text += collect_text(parser, name)?.as_str();
            if h_type != "normal" {
                text += "\\fR";
            }
        }
        "computeroutput" => {
            text += collect_text(parser, name)?.as_str();
        }
        "codeline" => {
            text += collect_text(parser, name)?.as_str();
        }
        "programlisting" => {
            text += "\n.nf\n";
            text += collect_text(parser, name)?.as_str();
            text += "\n.fi\n";
        }
        "itemizedlist" => {
            text += "\n";
            text += collect_text(parser, name)?.as_str();
            text += "\n";
        }
        "listitem" => {
            text += "\n* ";
            text += collect_text(parser, name)?.as_str();
        }
        "parameternamelist" => {
            text += collect_text(parser, name)?.as_str();
        }
        "parameteritem" => {
            text += collect_text(parser, name)?.as_str();
        }
        "parameterlist" => {
            text += collect_text(parser, name)?.as_str();
        }
        "parameterdescription" => {
            text += collect_text(parser, name)?.as_str();
        }
        "parametername" => {
            text += collect_text(parser, name)?.as_str();
        }
        "note" => {
            text += collect_text(parser, name)?.as_str();
            text += "\n";
        }
        "ref" => {
            text += collect_text(parser, name)?.as_str();
        }
        "simplesect" => {
            text += collect_text(parser, name)?.as_str();
        }
        "xreftitle" | "xrefdescription" | "xrefsect" => {
            let _ignore = collect_text(parser, name)?;
        }
        _ => {
        }
    }
    Ok(text)
}

// This returns the string itself (formatted) and a refid for the object if appropriate.
fn collect_text_and_refid(parser: &mut EventReader<BufReader<File>>) -> Result<(String, Option<String>), xml::reader::Error>
{
    let mut text = String::new();
    let mut refid = None;

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "ref" => {
                                refid = Some(get_attr(&e, "refid"));
                                text += collect_text(parser, name)?.as_str();
                            }
                            _ => {
                                text += parse_standard_elements(parser, name, &e)?.as_str();
                            }
                        }
                    }
                    XmlEvent::Characters(s) => {
                        text += s;
                    }
                    XmlEvent::EndElement {..} => {
                        return Ok((text.trim_end().to_string(), refid));
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

// Collect a single ReturnVal
fn collect_retval(parser: &mut EventReader<BufReader<File>>, elem_name: &OwnedName) -> Result<ReturnVal, xml::reader::Error>
{
    let mut ret_name = String::new();
    let mut ret_desc = String::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "parameternamelist" => {
                                ret_name = collect_text(parser, name)?.trim().to_string();
                            }
                            "parameterdescription" => {
                                ret_desc = collect_text(parser, name)?.trim().to_string();
                            }
                            _ => {
                                let _text = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::Characters(s) => {
                        let _text = s;
                    }
                    XmlEvent::EndElement {name, ..} => {
                        if name == elem_name {
                            return Ok(ReturnVal{ret_name, ret_desc})
                        };
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

// Collect all retvals for a function
fn collect_retvals(parser: &mut EventReader<BufReader<File>>, elem_name: &OwnedName) -> Result<Vec<ReturnVal>, xml::reader::Error>
{
    let mut rvs = Vec::<ReturnVal>::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "parameteritem" => {
                                rvs.push(collect_retval(parser, name)?);
                            }
                            _ => {
                                let _text = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::Characters(s) => {
                        let _text = s;
                    }
                    XmlEvent::EndElement {name, ..} => {
                        if name == elem_name {
                            return Ok(rvs)
                        };
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}


fn collect_parameter_item(parser: &mut EventReader<BufReader<File>>, elem_name: &OwnedName) -> Result<(String, String), xml::reader::Error>
{
    let mut par_name = String::new();
    let mut par_desc = String::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "parameternamelist" => {
                                par_name = collect_text(parser, name)?.trim().to_string();
                            }
                            "parameterdescription" => {
                                par_desc = collect_text(parser, name)?.trim().to_string();
                            }
                            _ => {
                                let _text = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::Characters(s) => {
                        let _text = s;
                    }
                    XmlEvent::EndElement {name, ..} => {
                        if name == elem_name {
                            return Ok((par_name, par_desc));
                        };
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

fn collect_params(parser: &mut EventReader<BufReader<File>>, elem_name: &OwnedName,
                  params: &mut Vec<FnParam>) -> Result<(), xml::reader::Error>
{
    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "parameteritem" => {
                                let (name, desc) = collect_parameter_item(parser, name)?;
                                // Add the desc to this param
                                for mut p in &mut *params {
                                    if p.par_name == name {
                                        p.par_desc = desc.clone();
                                    }
                                }
                            }
                            _ => {
                                let _text = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::Characters(s) => {
                        let _text = s;
                    }
                    XmlEvent::EndElement {name, ..} => {
                        if name == elem_name {
                            return Ok(())
                        };
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

}
// Called from "detaileddescription", so only needs to process tags that are immediately below it
// (everything below that is handled by collect_text()),
// and returns the main text, return text, and notes
fn collect_detail_bits(parser: &mut EventReader<BufReader<File>>,
                       elem_name: &OwnedName,
                       function: &mut FunctionInfo) -> Result<(), xml::reader::Error>
{
    let mut text = String::new();
    let mut returns = String::new();
    let mut notes = String::new();
    let mut retvals = Vec::<ReturnVal>::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "para" => {
                                collect_detail_bits(parser, name, function)?;
                                function.fn_detail += "\n";
                            }
                            "parameterlist" => {
                                if get_attr(&e, "kind") == "retval" {
                                    retvals = collect_retvals(parser, name)?;
                                } else if get_attr(&e, "kind") == "param" {
                                    collect_params(parser, name, &mut function.fn_args)?;
                                } else {
                                    text += collect_text(parser, name)?.as_str();
                                }
                            }
                            "simplesect" => {
                                if get_attr(&e, "kind") == "return" {
                                    returns += collect_text(parser, name)?.as_str();
                                } else if get_attr(&e, "kind") == "note" {
                                    notes += collect_text(parser, name)?.as_str();
                                } else  {
                                    text += collect_text(parser, name)?.as_str();
                                }
                            }
                            _ => {
                                text += parse_standard_elements(parser, name, &e)?.as_str();
                            }
                        }
                    }
                    XmlEvent::Characters(s) => {
                        text += s;
                    }
                    XmlEvent::EndElement {name, ..} => {
                        // Only return if we are at the end of the element that called us
                        if name == elem_name {
                            function.fn_detail += text.trim_end().to_string().as_str();
                            function.fn_returnval += returns.as_str();
                            function.fn_note += notes.as_str();
                            function.fn_retvals.append(&mut retvals);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

// This is the main text-collecting routine. It should parse as many XML options as possible.
// It returns the string itself (formatted).
// It is called recursively as we descend the XML structures
fn collect_text(parser: &mut EventReader<BufReader<File>>, elem_name: &OwnedName) -> Result<String, xml::reader::Error>
{
    let mut text = String::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        text += parse_standard_elements(parser, name, &e)?.as_str();
                    }
                    XmlEvent::Characters(s) => {
                        text += s;
                    }
                    XmlEvent::EndElement {name, ..} => {
                        // Only return if we are at the end of the element that called us
                        if name == elem_name {
                            return Ok(text.trim_end().to_string());
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

fn collect_function_param(parser: &mut EventReader<BufReader<File>>,
                          structures: &mut HashMap<String, StructureInfo>) -> Result<FnParam, xml::reader::Error>
{
    let mut par_name = String::new();
    let mut par_type = String::new();
    let mut par_refid = None;

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        let (tmp, refid) = collect_text_and_refid(parser)?;
                        if let Some(r) = &refid {
                            if structures.get(r).is_none() {
                                let new_struct = StructureInfo {str_type: StructureType::Struct, str_name: tmp.clone(), str_brief: String::new(), str_description: String::new(), str_members: Vec::<FnParam>::new()};
                                structures.insert(r.clone(), new_struct);
                            }
                        }

                        if name.to_string() == "type" {
                            par_type = tmp.clone();
                            par_refid = refid.clone();
                        }
                        if name.to_string() == "declname" {
                            par_name = tmp.clone();
                        }
                    }

                    XmlEvent::EndElement {..} => {
                        return Ok(FnParam{par_name, par_type, par_refid, par_args: String::new(), par_desc: String::new(), par_brief: String::new()});
                    }
                    _e => {
                    }
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

fn collect_function_info(parser: &mut EventReader<BufReader<File>>,
                         functions: &mut Vec<FunctionInfo>,
                         structures: &mut HashMap<String, StructureInfo>) -> Result<(), xml::reader::Error>
{
    let mut function = FunctionInfo::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "type" => {
                                function.fn_type = collect_text(parser, name)?;
                            },
                            "definition" =>  {
                                function.fn_def = collect_text(parser, name)?;
                            }
                            "argsstring" => {
                                function.fn_argsstring = collect_text(parser, name)?;
                            }
                            "name" | "compoundname" => {
                                function.fn_name = collect_text(parser, name)?;
                            }
                            "param" => {
                                let param = collect_function_param(parser, structures)?;
                                // If the param has a refid then make a note of it so we
                                // can expand structures in the manpage
                                if let Some(r) = &param.par_refid {
                                    function.fn_refids.push(r.clone());
                                }
                                function.fn_args.push(param);
                            }
                            "briefdescription" => {
                                function.fn_brief = collect_text(parser, name)?;
                            }
                            "detaileddescription" => {
                                collect_detail_bits(parser, name, &mut function)?;
                            }
                            _ => {
                                // Not used,. but still need to consume it
                                let _fntext = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::Characters(_s) => {

                    }
                    XmlEvent::EndElement {name, ..} => {
                        if name.to_string().as_str() == "memberdef" {
                            // Remove all duplicate refids for functions
                            // where a structure appears as multiple arguments
                            // (not common, but no need to print it twice)
                            function.fn_refids.sort_unstable();
                            function.fn_refids.dedup();

                            functions.push(function);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

fn collect_define(parser: &mut EventReader<BufReader<File>>) -> Result<HashDefine, xml::reader::Error>
{
    let mut hd_name = String::new();
    let mut hd_init = String::new();
    let mut hd_brief = String::new();
    let mut hd_desc = String::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "name" => {
                                hd_name = collect_text(parser, name)?;
                            }
                            "initializer" => {
                                hd_init = collect_text(parser, name)?;
                            }
                            "briefdescription" => {
                                hd_brief = collect_text(parser, name)?;
                            }
                            "detaileddescription" => {
                                hd_desc = collect_text(parser, name)?;
                            }
                            _ => {}
                        }
                    },
                    XmlEvent::EndElement {name, ..} => {
                        if name.to_string().as_str() == "memberdef" {
                            return Ok(HashDefine{hd_name, hd_init, hd_brief, hd_desc});
                        }
                    },
                    XmlEvent::Characters(_s) => {
                    },
                    XmlEvent::EndDocument => return Ok(HashDefine{hd_name, hd_init, hd_brief, hd_desc}),
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}


fn read_file(parser: &mut EventReader<BufReader<File>>,
             opt: &mut Opt,
             functions: &mut Vec<FunctionInfo>,
             structures: &mut HashMap<String, StructureInfo>) -> Result<(), xml::reader::Error>
{
    let mut defines = Vec::<HashDefine>::new();
    let mut general = FunctionInfo::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "memberdef" => {
                                if get_attr(&e, "kind") == "function" {

                                    // Do function stuff
                                    // go down the tree collecting info until we read EndElement
                                    collect_function_info(parser,
                                                          functions,
                                                          structures)?;
                                }
                                // Collect #defines
                                if get_attr(&e, "kind") == "define" {
                                    let new_hd = collect_define(parser)?;
                                    defines.push(new_hd);
                                }
                                // enums are in the main file, structs have their own
                                if get_attr(&e, "kind") == "enum" {
                                    let refid = get_attr(&e, "id");
                                    if let Ok(si) = collect_enum(parser, StructureType::Enum) {
                                        structures.insert(refid, si);
                                    }
				}
                                // Ignore typedefs for the moment
                                if get_attr(&e, "kind") == "typedef" {
                                    let _ignore = collect_text(parser, name)?;
                                }
                            }
                            "compoundname" => {
                                // This is the header filename (and the reason &opt is mutable & cloned)
				if opt.headerfile == "unknown.h" {
                                    opt.headerfile = collect_text(parser, name)?;
				}
                            }

                            // These are at the file (eg qblog.h) level
                            "briefdescription" => {
                                general.fn_brief += collect_text(parser, name)?.as_str();
                            }
                            "detaileddescription" => {
                                collect_detail_bits(parser, name, &mut general)?;
                            }
                            _ => {
                                let _tother = parse_standard_elements(parser, name, &e)?;
                            }
                        }
                    },
                    XmlEvent::EndElement {..} => {
                    },
                    XmlEvent::Characters(_s) => {
                    },
                    XmlEvent::EndDocument => {
                        general.fn_name = opt.headerfile.clone();
                        general.fn_defines = defines;
                        functions.push(general);
                        return Ok(());
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

// Read a single structure member from a structure file
fn read_structure_member(parser: &mut EventReader<BufReader<File>>) -> Result<FnParam, xml::reader::Error>
{
    let mut par_name = String::new();
    let mut par_type = String::new();
    let mut par_desc = String::new();
    let mut par_brief = String::new();
    let mut par_args = String::new();

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "name" => {
                                par_name = collect_text(parser, name)?;
                            }
                            "type" => {
                                par_type = collect_text(parser, name)?;
                            }
                            "argsstring" => {
                                par_args = collect_text(parser, name)?;
                            }
                            "detaileddescription" => {
                                par_desc = collect_text(parser, name)?.trim().to_string();
                            }
                            "briefdescription" => {
                                par_brief = collect_text(parser, name)?.trim().to_string();
                            }
                            _ => {
                                // Not used but still needs to be collected
                                let _fntext = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::EndElement {..} => {
                        return Ok(FnParam {par_name, par_type, par_desc, par_args, par_brief, par_refid: None});
                    },
                    XmlEvent::Characters(_s) => {
                    },
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

fn collect_enum(parser: &mut EventReader<BufReader<File>>,
                str_type: StructureType) -> Result<StructureInfo, xml::reader::Error>
{
    let mut sinfo = StructureInfo::new();
    sinfo.str_type = str_type;

    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "name" => {
                                sinfo.str_name = collect_text(parser, name)?;
                            }
                            "enumvalue" => {
                                match read_structure_member(parser) {
                                    Ok(s) => sinfo.str_members.push(s),
                                    Err(e) => return Err(e),
                                }
                            }
                            "briefdescription" => {
                                sinfo.str_brief = collect_text(parser, name)?;
                            }
                            "detaileddescription" => {
                                sinfo.str_description = collect_text(parser, name)?;
                            }
                            _ => {
                                let _ = collect_text(parser, name)?;
                            }
                        }
                    }
                    XmlEvent::EndElement {..} => {
                        return Ok(sinfo);
                    },
                    XmlEvent::Characters(_s) => {
                    },
                    XmlEvent::EndDocument => return Ok(sinfo),
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}


// Found the point in the struct file where the definition is. Read it in
fn read_structure(parser: &mut EventReader<BufReader<File>>,
                  str_type: StructureType) -> Result<StructureInfo, xml::reader::Error>
{
    let mut sinfo = StructureInfo::new();

    sinfo.str_type = str_type;
    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "compoundname" => {
                                sinfo.str_name = collect_text(parser, name)?;
                            }
                            "briefdescription" => {
                                sinfo.str_brief = collect_text(parser, name)?;
                            }
                            "includes" => {
                                let _ignore = collect_text(parser, name)?;
                            }
                            "detaileddescription" => {
                                sinfo.str_description = collect_text(parser, name)?;
                            }
                            "memberdef" => {
                                match read_structure_member(parser) {
                                    Ok(s) => sinfo.str_members.push(s),
                                    Err(e) => return Err(e),
                                }
                            }
                            _ => {}
                        }
                    }
                    XmlEvent::EndElement {name, ..} => {
                        if name.to_string() == "compounddef" {
                            return Ok(sinfo);
                        }
                    },
                    XmlEvent::Characters(_s) => {
                    },
                    XmlEvent::EndDocument => {},
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}

// Read a single structure from its XML file
fn read_structure_file(parser: &mut EventReader<BufReader<File>>,
                       str_type: StructureType) -> Result<(String, StructureInfo), xml::reader::Error>
{
    let mut sinfo = StructureInfo::new();
    let mut refid = String::new();

    sinfo.str_type = str_type;
    loop {
        let er = parser.next();
        match er {
            Ok(e) => {
                match &e {
                    XmlEvent::StartElement {name, ..} => {
                        match name.to_string().as_str() {
                            "compounddef" => {
                                if let Ok(s) = read_structure(parser, StructureType::Struct) {
                                    sinfo = s;
                                    refid = get_attr(&e, "id");
                                }
                            }
                            "briefdescription" => {
                                sinfo.str_brief = collect_text(parser, name)?;
                            }
                            "detaileddescription" => {
                                sinfo.str_description = collect_text(parser, name)?;
                            }
                            _ => {}
                        }
                    }
                    XmlEvent::EndElement {..} => {
                    },
                    XmlEvent::Characters(_s) => {
                    },
                    XmlEvent::EndDocument => return Ok((refid, sinfo)),
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
}


// Read all the structure files we need for our functions
fn read_structures_files(opt: &Opt,
                         structures: &HashMap<String, StructureInfo>,
                         filled_structures: &mut HashMap<String, StructureInfo>)
{
    for (refid, s) in structures {
        match s.str_type {
            StructureType::Enum => {
                filled_structures.insert(refid.to_string(), (*s).clone());
            }
            StructureType::Unknown => {} // Throw it away
            StructureType::Struct => {
                let mut xml_file = String::new();
                if let Err(e) = write!(xml_file, "{}/{}.xml", &opt.xml_dir, &refid) {
                    println!("Error making structure XML file name for {refid}: {e}");
                    return;
                }

                if let Ok(f) = File::open(&xml_file) {
                        let mut parser = ParserConfig::new()
                            .whitespace_to_characters(true)
                            .ignore_comments(true)
                            .create_reader(BufReader::new(f));

                    if let Ok((refid, new_s)) = read_structure_file(&mut parser, StructureType::Struct) {
                        // Add to the new map
                        filled_structures.insert(refid, new_s);
                    }
		}
            }
        }
    }
}

fn read_header_copyright(opt: &Opt) -> Result<String, std::io::Error>
{
    let mut h_file = String::new();
    if let Err(_e) = write!(h_file, "{}/{}", &opt.header_src_dir, &opt.headerfile) {
        println!("Error making header file name for {}: {}", opt.header_src_dir, opt.headerfile);
        return Err(Error::new(ErrorKind::Other, "Error making filename"));
    }

    let f = File::open(&h_file)?;
    let r = BufReader::new(f);
    for l in r.lines() {
        match l {
            Ok(line) => {
                if line.starts_with(" * Copyright") {
                    // unwrap is safe here because of the above line.
                    return Ok(line.get(3..).unwrap().to_string());
                }
            }
            Err(e) => return Err(e)
        }
    }
    Err(Error::new(ErrorKind::Other, "Not found"))
}


// Mainly for debugging
fn print_text_function(f: &FunctionInfo,
                       structures: &HashMap<String, StructureInfo>)
{
    println!("FUNCTION {} {} {}", f.fn_type, f.fn_name, f.fn_argsstring);
    for i in &f.fn_args {
        match &i.par_refid {
            Some(r) =>
                println!("  PARAM: {} {}{} (refid={})", i.par_type, i.par_name, i.par_args, r),
            None =>
                println!("  PARAM: {} {}{}", i.par_type, i.par_name, i.par_args),
        }
        if !i.par_brief.is_empty() {
            println!("  PARAM brief: {}", i.par_brief);
        }
        if !i.par_desc.is_empty() {
            println!("  PARAM desc: {}", i.par_desc);
        }
    }
    println!("BRIEF: {}", f.fn_brief);
    println!("DETAIL: {}", f.fn_detail);


    for fs in &f.fn_refids {
        if let Some(s) = structures.get(fs) {
            println!("STRUCTURE: {}", s.str_name);
            if !s.str_brief.is_empty() {
                println!("           {}", s.str_brief);
            }
            if !s.str_description.is_empty() {
                println!("           {}", s.str_description);
            }
            for m in &s.str_members {
                println!("   MEMB: {} {}{}", m.par_type, m.par_name, m.par_args);
            }
        }
    }

    println!("----------------------");
}

// Format a long description string
fn print_long_string(f: &mut BufWriter<File>, s: &str) -> Result<(), std::io::Error>
{
    let mut in_nf = false;

    // Check for .nf / .fi and don't format those!
    for l in s.lines() {
        if l.starts_with(".nf") {
            writeln!(f)?;
            in_nf = true;
        }

        writeln!(f,"{l}")?;

        if !in_nf {
            writeln!(f,".PP")?;
        }

        if l.starts_with(".fi") {
            writeln!(f)?;
            in_nf = false;
        }
    }
    Ok(())
}

// Just for testing really
fn print_ascii_pages(_opt: &Opt,
                     functions: &[FunctionInfo],
                     structures: &HashMap<String, StructureInfo>)
{
    for f in functions {
        print_text_function(f, structures);
    }
}


fn print_long_structure_comment(f: &mut BufWriter<File>, comment: &str) -> Result<(), std::io::Error>
{
    writeln!(f, "    \\fP/*")?;
    write!(f, "     *")?;

    let mut column = 7;
    for word in comment.split_whitespace() {
	column += word.len();
	if column > 80 {
	    write!(f, "\n     *")?;
	    column = 7;
	}
	write!(f, " {word}")?;
    }
    writeln!(f, "\n     */")?;
    Ok(())
}

// Prints a structure member or a function param given
// a field width. Also reformats pointers to look nicer (IMHO)
fn print_param(f: &mut BufWriter<File>, pi: &FnParam, type_field_width: usize,
	       name_field_width: usize, bold: bool, delimeter: String) -> Result<(), std::io::Error>
{
    let mut asterisks = "  ".to_string();
    let mut formatted_type = pi.par_type.clone();
    let typelen: usize = formatted_type.len();

    // Reformat pointer params so they look nicer
    // these unwrap()s are safe because we check the length before doing the get()
    if !formatted_type.is_empty() && formatted_type.get(typelen-1..typelen).unwrap() == "*" {
        asterisks = " *".to_string();
        formatted_type = pi.par_type.get(..typelen-1).unwrap().to_string();

        // Cope with double pointers
        if typelen > 1 && formatted_type.get(typelen-2..typelen-1).unwrap() == "*" {
            asterisks = "**".to_string();
            formatted_type = pi.par_type.get(..typelen-2).unwrap().to_string();
        } else {
            // Tidy function pointers
            if typelen > 1 && formatted_type.get(typelen-2..typelen-1).unwrap() == "(" {
                asterisks = "(*".to_string();
                formatted_type = pi.par_type.get(..typelen-2).unwrap().to_string();
            }
	}
    }

    // Put long comments on their own line for clarity
    let comment_len = len_without_formatting(&pi.par_desc);
    if comment_len > MAX_STRUCT_COMMENT_LEN {
	print_long_structure_comment(f, &pi.par_desc)?;
    }

    if bold {
        write!(f, "    \\fB")?;
    } else {
        write!(f, "    \\fR")?;
    }
    write!(f, "{:<width$}{}\\fI{}\\fB{}\\fR{}",
           formatted_type, asterisks,
           pi.par_name, pi.par_args, delimeter, width=type_field_width)?;

    // Field description */
    if comment_len > 0 && comment_len <= MAX_STRUCT_COMMENT_LEN && name_field_width > 0 {
	let pad_width = 1 + (name_field_width - pi.par_name.len() - pi.par_args.len()) - delimeter.len();
	write!(f, "\\fP {:>width$} /* {} */", "", pi.par_desc, width=pad_width)?;
    }
    writeln!(f)?;
    Ok(())
}

// Print a structure or enum
fn print_structure(f: &mut BufWriter<File>, si: &StructureInfo) -> Result<(), std::io::Error>
{
    if !si.str_brief.is_empty() {
        writeln!(f, "{}", si.str_brief)?;
    }
    if !si.str_description.is_empty() {
        writeln!(f, "{}", si.str_description)?;
    }

    let mut max_param_type_length = 0;
    let mut max_param_name_length = 0;
    for p in &si.str_members {
        if p.par_type.len() > max_param_type_length {
            max_param_type_length = p.par_type.len();
	}
        if p.par_name.len() + p.par_args.len() > max_param_name_length {
            max_param_name_length = p.par_name.len() + p.par_args.len();
        }
    }

    writeln!(f,)?;
    writeln!(f, ".nf")?;
    writeln!(f, "\\fB")?;
    match si.str_type {
        StructureType::Enum =>  writeln!(f, "enum {} {{", si.str_name)?,
        StructureType::Struct => writeln!(f, "struct {} {{", si.str_name)?,
        StructureType::Unknown => writeln!(f, "??? {} {{", si.str_name)?,
    };

    let mut i=0;
    for p in &si.str_members {
        i += 1;
        if i == si.str_members.len() {
            print_param(f, p, max_param_type_length, max_param_name_length, false, "".to_string())?;
        } else {
            print_param(f, p, max_param_type_length, max_param_name_length, false, ";".to_string())?;
        }
    }

    writeln!(f, "}};\\fP")?;
    writeln!(f, ".PP")?;
    writeln!(f, ".fi")?;

    Ok(())
}

// Print a single man page
fn print_man_page(opt: &Opt,
                  man_date: &str,
                  function: &FunctionInfo,
                  functions: &[FunctionInfo],
                  structures: &HashMap<String, StructureInfo>,
                  copyright: &str) -> Result<(), std::io::Error>
{
    if function.fn_name == opt.headerfile && !opt.print_general {
        return Ok(());
    }

    // DO IT!
    let mut man_file = String::new();
    if let Err(e) = write!(man_file, "{}/{}.{}", &opt.output_dir, function.fn_name, opt.man_section) {
        eprintln!("Error making manpage filename: {e:?}");
        return Err(Error::new(ErrorKind::Other, "Error making filename"));
    }

    let dateptr = man_date;

    match File::create(&man_file) {
        Err(e) => {
            println!("Cannot create man file {}: {}", &man_file, e);
            return Err(e);
        }
        Ok(fl) => {
            let mut f = BufWriter::new(fl);

            // Work out the length of the parameters, so we can line them up
            let mut max_param_type_len: usize = 0;
            let mut max_param_name_len: usize = 0;
            let mut num_param_descs: usize = 0;
            let mut param_count: usize = 0;

            for p in &function.fn_args {
                if (p.par_type.len() < MAX_PRINT_PARAM_LEN) &&
                    (p.par_type.len() > max_param_type_len) {
                        max_param_type_len = p.par_type.len();
                    }
                if p.par_name.len() > max_param_name_len {
                    max_param_name_len = p.par_name.len();
                }
                if !p.par_desc.is_empty() && !p.par_type.is_empty() {
                    num_param_descs += 1;
                }
                param_count += 1;
            }

            writeln!(f, ".\\\"  Automatically generated man page, do not edit")?;
            writeln!(f, ".TH {} {} {} \"{}\" \"{}\"",
                     function.fn_name.to_ascii_uppercase(), opt.man_section, dateptr, opt.package_name, opt.header)?;

            writeln!(f, ".SH NAME")?;
            writeln!(f, ".PP")?;
            if !function.fn_brief.is_empty()  {
                writeln!(f, "{} \\- {}", function.fn_name, function.fn_brief)?;
            } else {
                writeln!(f, "{}", function.fn_name)?;
            }

            writeln!(f, ".SH SYNOPSIS")?;
            writeln!(f, ".PP")?;
	    writeln!(f, ".nf")?;
	    writeln!(f, ".B #include <{}{}>", opt.header_prefix, opt.headerfile)?;
            if !function.fn_def.is_empty() {
                writeln!(f, ".sp")?;
                writeln!(f, "\\fB{}\\fP(", function.fn_def)?;

                let mut i=0;
                for p in &function.fn_args {
                    i += 1;
                    if i == param_count {
                        print_param(&mut f, p, max_param_type_len, 0, true, "".to_string())?;
                    } else {
                        print_param(&mut f, p, max_param_type_len, 0, true, ",".to_string())?;
                    }
                }

                writeln!(f, ");")?;
                writeln!(f, ".fi")?;
            }

            if opt.print_params && num_param_descs > 0 {
	        writeln!(f, ".SH PARAMETERS")?;
                writeln!(f, ".PP")?;
                for p in &function.fn_args {
                    writeln!(f, ".TP")?;
                    writeln!(f, "\\fB{}\\fP {}",
                             p.par_name, p.par_desc)?;
                }
            }
            if !function.fn_detail.is_empty() {
	        writeln!(f, ".SH DESCRIPTION")?;
                writeln!(f, ".PP")?;
                print_long_string(&mut f, &function.fn_detail)?;
            }

            if !function.fn_refids.is_empty() {
                let mut first = true; // In case we can't find the refids, don't print the header

                for fs in &function.fn_refids {
                    if let Some(s) = structures.get(fs) {
                        if first {
                            writeln!(f, ".SH STRUCTURES")?;
                            writeln!(f, ".PP")?;
                            first = false;
                        }
                        print_structure(&mut f, s)?;
                    }
                }
            }
            if !function.fn_returnval.is_empty() {
	        writeln!(f, ".SH RETURN VALUE")?;
                writeln!(f, ".PP")?;
                writeln!(f, "{}", function.fn_returnval)?;
                writeln!(f, ".br")?;
                for rv in &function.fn_retvals {
                    writeln!(f, ".TP")?;
                    writeln!(f, "\\fB{}\\fR {}", rv.ret_name, rv.ret_desc)?;
                }
                writeln!(f, ".PP")?;
            }

            // #defines - only exists on the General manpage
            if !function.fn_defines.is_empty() {
                writeln!(f, ".SH DEFINES")?;
                writeln!(f, ".PP")?;
                for d in &function.fn_defines {
                    // Only print ALLCAPS defines, for neatness
                    if d.hd_name == d.hd_name.to_ascii_uppercase() {
                        if !d.hd_brief.is_empty() {
                            writeln!(f, ".PP")?;
                            writeln!(f, "{}", d.hd_brief)?;
                            writeln!(f, ".br")?;
                        }
                        if !d.hd_desc.is_empty() {
                            writeln!(f, ".br")?;
                            writeln!(f, "{}", d.hd_desc)?;
                            writeln!(f, ".br")?;
                        }

                        writeln!(f, "#define {} {}", d.hd_name, d.hd_init)?;
                        writeln!(f, ".br")?;
                    }
                }
            }

            if !function.fn_note.is_empty() {
	        writeln!(f, ".SH NOTE")?;
                writeln!(f, ".PP")?;
                print_long_string(&mut f, &function.fn_note)?;
            }

            // Print list of related functions
	    writeln!(f, ".SH SEE ALSO")?;
	    writeln!(f, ".PP")?;
	    writeln!(f, ".nh")?;
	    writeln!(f, ".ad l")?;
            let mut num_func = 0;
            for func in functions {
                num_func += 1;
                if func.fn_name != function.fn_name {
                    let delim =
                        if num_func == functions.len() {
                            ""
                        } else {
                            ", "
                        };
	            writeln!(f, "\\fI{}\\fP({}){}", func.fn_name, opt.man_section, delim)?;
                };
            }

            if !copyright.is_empty() {
                writeln!(f, ".SH COPYRIGHT")?;
                writeln!(f, ".PP")?;
                writeln!(f,"{copyright}")?;
            }

            //END OF PRINTING
        }
    }
    Ok(())
}


// Print all man pages
fn print_man_pages(opt: &Opt,
                   functions: &[FunctionInfo],
                   structures: &HashMap<String, StructureInfo>) -> Result<(), std::fmt::Error>
{
    let mut date_to_print = String::new();
    let mut header_copyright = String::new();
    let mut manpage_year: i32 = opt.manpage_year;

    // Get current date
    let today: DateTime<Local> = Local::now();

    if !opt.manpage_date.is_empty() {
        date_to_print = opt.manpage_date.clone();
    } else {
        write!(date_to_print, "{}-{}-{}", today.year(), today.month(), today.day())?;
    }

    if manpage_year == 0 {
        manpage_year = today.year();
    }

    if opt.use_header_copyright {
        if let Ok(s) = read_header_copyright(opt) {
            header_copyright = s;
        }
    } else {
        write!(header_copyright, "Copyright (C) {}-{} {}, All rights reserved",
               opt.start_year, manpage_year, opt.company)?;
    }

    for f in functions {
        print_man_page(opt, &date_to_print, f, functions, structures, &header_copyright).unwrap();
    }
    Ok(())
}


fn main() {

    // Get command-line options
    let mut opt = Opt::from_args();

    for in_file in &opt.xml_files.clone() {
        let mut main_xml_file = String::new();
        if let Err(e) = write!(main_xml_file, "{}/{}", &opt.xml_dir, &in_file) {
            eprintln!("Error making main XML file name for {in_file}: {e}");
            return;
        }

        match File::open(&main_xml_file) {
            Ok(f) => {
                let mut parser = ParserConfig::new()
                    .whitespace_to_characters(true)
                    .ignore_comments(true)
                    .create_reader(BufReader::new(f));

                let mut functions = Vec::<FunctionInfo>::new();
                let mut structures = HashMap::<String, StructureInfo>::new();

                // Read it all into structures
                if let Err(e) = read_file(&mut parser, &mut opt, &mut functions, &mut structures) {
                    eprintln!("Error reading XML for {main_xml_file}: {e:?}");
                    continue;
                }

                // Go through the structures map and read those files in to get the full structure info
                let mut filled_structures = HashMap::<String, StructureInfo>::new();
                read_structures_files(&opt, &structures,
                                      &mut filled_structures);

                // Then print those man pages!
                if opt.print_ascii {
                    print_ascii_pages(&opt, &functions, &filled_structures);
                }
                if opt.print_man {
                    if let Err(e) = print_man_pages(&opt, &functions, &filled_structures) {
                        eprintln!("Error in print_man_pages: {e:?}");
                        break;
                    }
                }
            }
            Err(e) => {
                println!("Cannot open XML file {}: {}", &main_xml_file, e);
            }
        }
    }
}
