/* M39.3 - FMTLIB : liste le contenu du catalogue WORK. Oracle : la liste des
   formats/informats affiches = exactement ceux definis ci-dessous, ni plus
   ni moins - un VALUE numerique (AGEF), un VALUE caractere ($GRADEF), et un
   INVALUE (GRADEI). */

proc format fmtlib;
  value agef low-<21='Minor' 21-high='Adult' other='Unknown';
  value $gradef 'A'='Excellent' 'B'='Good' 'C'='Average';
  invalue gradei 'A'=4 'B'=3 'C'=2 other=0;
run;
