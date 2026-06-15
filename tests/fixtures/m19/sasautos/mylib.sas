/* M19.2 - autocall library (SASAUTOS).                                   */
/* This macro is NOT defined in macro_advanced.sas: it is searched for    */
/* as mylib.sas in the SASAUTOS directory and compiled lazily on first    */
/* invocation.                                                            */
/*                                                                          */
/* NB: the snapshot harness executes each fixture in a fresh tempdir and   */
/* does not copy sibling files; macro_advanced.sas therefore writes an     */
/* equivalent copy of this macro at build time (via byte()) and points     */
/* SASAUTOS at it. This file documents the canonical expected form.        */
%macro mylib(value=);
    data work.auto; v = &value; output; run;
%mend;
