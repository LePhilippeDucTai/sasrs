title 'M29.3: UNIVARIATE plots + REG diagnostics via ODS GRAPHICS';

data work.normal_data;
  do i = 1 to 20;
    x = i + (i - 10) * 0.5;
    output;
  end;
  drop i;
run;

/* Test 1: UNIVARIATE HISTOGRAM sans ODS ON */
proc univariate data=work.normal_data noprint;
  var x;
  histogram x;
run;

/* Test 2: avec ODS GRAPHICS ON */
ods graphics on;

proc univariate data=work.normal_data noprint;
  var x;
  histogram x;
  qqplot x;
run;

proc reg data=work.normal_data;
  model x = ;
run;

ods graphics off;

title;
