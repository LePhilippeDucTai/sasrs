/* M35.4 - macro abort halts the macro body and the rest of the submission. */
%put before_macro;
%macro stopper;
  %put in_macro;
  %abort;
  %put never_in_macro;
%mend stopper;
%stopper
%put never_open_code;
