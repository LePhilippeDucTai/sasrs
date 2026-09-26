/* Oracle: SAS Macro Language Reference (%EVAL, %SYSEVALF, %SYSFUNC).
   https://support.sas.com/documentation/cdl/en/mcrolref/67912/HTML/default/titlepage.htm
   Parameter diagnostics: https://support.sas.com/kb/31/012.html
   https://support.sas.com/kb/43764.html */
%put BEFORE;
%let bad=%eval(1/0);
%let bad=%sysevalf(1/0);
%let bad=%sysfunc(j02_unknown(1));
%macro args(a,k=); %put NEVER; %mend;
%args(1,2)
%args(1,unknown=2)
%macro jump; %goto missing; %mend;
%jump
%put AFTER;
