// Associative arrays and queues whose elements are queues or associative
// arrays (IEEE 1800-2017 7.2), from a typedef or declared with several
// dimensions: element methods, element reads and writes, copies (arrays are
// values), foreach over all dimensions; reading a missing key creates none.
module top;
  typedef enum {X, Y, Z} e_t;
  class obj; endclass
  e_t states[obj][obj][int];
  int qs[string][$];
  typedef int q_t[$];
  q_t all[int];
  q_t by_name[string];
  q_t qq[$];
  q_t q, r;
  int fails = 0;

  task check(input bit ok, input string what);
    if (!ok) begin
      $display("FAILED: %s", what);
      fails++;
    end
  endtask

  function automatic int total(input int unsigned n);
    q_t local_all[int];
    int s = 0;
    for (int i = 0; i < n; i++) local_all[i % 3].push_back(i);
    foreach (local_all[k, j]) s += local_all[k][j];
    return s;
  endfunction

  initial begin
    all[3].push_back(30);
    all[3].push_back(31);
    all[1].push_front(10);
    by_name["a"].push_back(5);
    check(all.num() == 2 && all[3].size() == 2 && all[1][0] == 10 && all[3][1] == 31, "push and read");
    check(by_name["a"].size() == 1 && by_name.exists("a"), "string keys");
    check(all[7].size() == 0 && all.num() == 2, "a missing key reads as empty, creates none");
    all[3][0] = 33;
    check(all[3][0] == 33, "element of an element written");
    q = all[3];
    q.push_back(32);
    check(q.size() == 3 && all[3].size() == 2, "a read copies");
    q = '{1, 2};
    all[5] = q;
    q[0] = 9;
    check(all[5][0] == 1 && all.num() == 3, "a write copies");
    foreach (all[i, j]) r.push_back(all[i][j]);
    check(r.size() == 5 && r[0] == 10 && r[1] == 33 && r[2] == 31 && r[3] == 1 && r[4] == 2, "foreach over both dimensions");
    r = {r, all[1]};
    check(r.size() == 6 && r[5] == 10, "an element concatenated");
    all[5].delete();
    check(all[5].size() == 0 && all.num() == 3, "an element emptied");
    all.delete(5);
    check(all.num() == 2, "an element removed");
    qq.push_back(q);
    qq[0].push_back(7);
    check(qq.size() == 1 && qq[0].size() == 3 && qq[0][2] == 7 && q.size() == 2, "a queue of queues");
    check(total(10) == 45, "an automatic array of queues");
    begin
      obj a = new, b = new;
      int n = 0;
      states[a][b][1] = Y;
      states[a][a][2] = Z;
      check(states.num() == 1 && states[a].num() == 2 && states[a][b][1] == Y, "three associative levels");
      check(states[b].num() == 0 && states.num() == 1 && states[a][a].exists(2), "missing keys of three levels");
      foreach (states[i, j, k]) n++;
      check(n == 2, "foreach over three levels");
      qs["x"].push_back(4);
      qs["x"].push_back(5);
      check(qs["x"].size() == 2 && qs["x"][1] == 5, "int qs[string][$]");
    end
    if (fails == 0) $display("PASSED");
  end
endmodule
