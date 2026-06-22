/* M35.4 - macro control flow: early-return and goto/label loop. */
%macro early;
  %put early_before;
  %return;
  %put early_after_should_not_appear;
%mend early;
%early

%macro countup;
  %local i;
  %let i = 0;
  %top:
  %let i = %eval(&i + 1);
  %put count=&i;
  %if &i < 3 %then %goto top;
  %put countup_done;
%mend countup;
%countup
%put after_all;
