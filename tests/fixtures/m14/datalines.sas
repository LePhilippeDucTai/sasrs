/* M14.1 : INFILE INPUT avec DATALINES et modes liste / colonne. */

* Mode liste : lecture des champs séparés par des espaces. ;
data work.scores;
  input name $ score;
  datalines;
Alice 95
Bob 82
Carol 78
;
run;

proc print data=work.scores;
  title 'DATALINES list mode';
run;

* Mode colonne : positions fixes. ;
data work.cols;
  input name $ 1-5 score 7-9;
  datalines;
Alice  95
Bob    82
Carol  78
;
run;

proc print data=work.cols;
  title 'DATALINES column mode';
run;

* Données manquantes avec MISSOVER. ;
data work.missover;
  infile datalines missover;
  input a b c;
  datalines;
1 2 3
4 5
7
;
run;

proc print data=work.missover;
  title 'MISSOVER — short lines fill with missing';
run;
