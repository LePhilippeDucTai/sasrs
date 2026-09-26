/* Oracle: https://support.sas.com/kb/23/addl/fusion23211_1_abort.html
   RETURN n stops processing and returns n to the caller. */
%macro stop; %abort return 8; %mend;
%stop
data _null_; put 'NEVER'; run;
%put LATER;
