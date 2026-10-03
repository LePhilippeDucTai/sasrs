/* Issue #20 — reproducer de déterminisme.
 *
 * Deux sources de non-déterminisme, en un seul programme :
 *  1. PROC SQL UNION (non-ALL) : concat + unique() de Polars → l'ORDRE des
 *     lignes du résultat n'était pas reproductible d'une exécution à
 *     l'autre (ex æquo conservés dans un ordre arbitraire).
 *  2. Sidecar <t>.parquet.sasmeta.json : sérialisé depuis une HashMap →
 *     l'ordre des clés JSON variait PAR PROCESSUS.
 *
 * Le programme écrit out.unioned (Parquet + sidecar : formats/libellés
 * déclarés ⇒ has_meta) puis l'imprime ; run_check.sh compare tous les
 * artefacts byte-à-byte sur 5 exécutions.
 */
libname out 'out';

data work.a;
    length name $8;
    format wt 8.1;
    label name = 'Nom' wt = 'Poids (kg)';
    input name $ age wt;
    datalines;
Alfred 14 112.5
Alice 13 84.0
Carol 14 62.8
David 15 99.0
;

data work.b;
    length name $8;
    format wt 8.1;
    label name = 'Nom' wt = 'Poids (kg)';
    input name $ age wt;
    datalines;
Carol 14 62.8
Jane 12 74.2
Alfred 14 112.5
Bob 13 90.5
;

proc sql;
    create table out.unioned as
        select name, age, wt from work.a
        union
        select name, age, wt from work.b;
quit;

proc print data=out.unioned;
run;
