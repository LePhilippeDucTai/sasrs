/* M12: conditional macro loop (do-while) generating rows; upto(4) makes x=1..4. */
%macro upto(n);
  %let i = 1;
  %do %while(&i <= &n);
    x = &i; output;
    %let i = %eval(&i + 1);
  %end;
%mend;

data nums;
  %upto(4)
run;

title 'Rows via macro do-while';
proc print data=nums;
  var x;
run;
