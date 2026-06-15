/* M19 - macro advanced: unquote, include, autocall (SASAUTOS), put,    */
/* MPRINT/MLOGIC/SYMBOLGEN trace, call execute (macro-side).            */
/*                                                                       */
/* Implementation notes:                                                 */
/*  - the macro processor resolves macro triggers EVERYWHERE (string     */
/*    literals AND comments), so the .sas files written at build time for */
/*    include and SASAUTOS are assembled via byte(37) and byte(38), and  */
/*    the comments below avoid those two sigils entirely;                */
/*  - macro expansion is interleaved per step (the segmenter cuts on the */
/*    step keywords, which comments must also avoid), so an option that  */
/*    must be visible to a later macro (SASAUTOS, the trace flags) is set */
/*    in an EARLIER step, separated by a step boundary from the macro it  */
/*    governs.                                                            */

/* ---- M19.1 : macro unquote --------------------------------------------- */
/* nrstr masks the symbol reference; unquote re-activates it, so the     */
/* nested symbol is then resolved.                                       */
%let name = Alice;
%let quoted = %nrstr(Hello &name);
%let result = %unquote(&quoted);
%put M19.1 unquoted: &result;

/* ---- M19.2 : include - a .sas file that DEFINES a macro ---------------- */
/* Write incmac.sas (relative to base_dir), include it (registers        */
/* get_name), then invoke the macro.                                     */
data _null_;
    file 'incmac.sas';
    length line $200;
    line = byte(37) || 'macro get_name(id);';                            put line;
    line = '  data work.gn; nm = "' || byte(38) || 'id"; output; run;';  put line;
    line = byte(37) || 'mend;';                                          put line;
run;

%include 'incmac.sas';
%get_name(SAS);

/* ---- M19.2 : autocall (SASAUTOS) - lazy compilation -------------------- */
/* Write mylib.sas, point SASAUTOS at it, then (after the step boundary)  */
/* invoke mylib: undefined in code, it is searched for and compiled       */
/* lazily from disk.                                                      */
data _null_;
    file 'mylib.sas';
    length line $200;
    line = byte(37) || 'macro mylib(value=);';                              put line;
    line = '  data work.auto; v = ' || byte(38) || 'value; output; run;';   put line;
    line = byte(37) || 'mend;';                                             put line;
run;

options sasautos='.';
data _null_; run;
%mylib(value=42)

/* ---- M19.3 : put + MPRINT/MLOGIC/SYMBOLGEN ----------------------------- */
/* Set the trace flags, then close the step so the next macro is          */
/* expanded with the flags ON.                                            */
options mprint mlogic symbolgen;
data _null_; run;

%put The name is: &name;
title "Report for &name";

%macro test_logic(n);
    %if &n = 3 %then %put N is three;
    %else %put N is not three;
%mend;
%test_logic(3)

options nomprint nomlogic nosymbolgen;
title;
data _null_; run;

/* ---- M19.3 : call execute (macro-side) - deferred execution ------------ */
/* defer queues a DATA step; it executes at the end of THIS step, so      */
/* work.result exists before the proc print that follows.                 */
%macro defer;
    %call execute(data work.result; x = 99; output; run;);
%mend;
%defer
data _null_; run;

/* ---- Final outputs ----------------------------------------------------- */
proc print data=work.gn;     run;
proc print data=work.auto;   run;
proc print data=work.result; run;
