/* Oracle: SAS Macro Language Reference, %NRSTR and %LENGTH.
   https://support.sas.com/documentation/cdl/en/mcrolref/61885/HTML/default/a001061290.htm
   https://support.sas.com/documentation/cdl/en/mcrolref/61885/HTML/default/a000543620.htm */
%put &missing;
%put %unknown;
%put %nrstr(&protected %protected);
%let empty=;
%put [%length()][%length(&empty)];
%sysexec ignored;
%window w;
%syscall unsupported(x);
