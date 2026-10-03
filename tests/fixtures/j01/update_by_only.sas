/* J01-P1 — UPDATE sans KEY= (issue #13) : la doc SAS n'exige qu'un
 * statement BY (UPDATE Statement,
 * https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lestmtsref/n0zi0al7gygfzmn12ga0djr39d77).
 * Les variables BY servent de clés de correspondance maître/transaction ;
 * une transaction sans maître est AJOUTÉE (LEPG ch. 21) ; une valeur
 * manquante de transaction reste sans effet (MISSINGCHECK par défaut).
 *
 * Sortie attendue :
 *   1 100  (pas de transaction pour id=1 → maître inchangé)
 *   2 250
 *   3 330
 *   4 400  (transaction sans maître → nouvelle observation)
 */

data work.master;
  input id price;
  datalines;
1 100
2 200
3 300
;
run;

data work.trans;
  input id price;
  datalines;
2 250
3 330
4 400
;
run;

data work.master;
  update work.master work.trans;
  by id;
run;

proc print data=work.master;
run;
