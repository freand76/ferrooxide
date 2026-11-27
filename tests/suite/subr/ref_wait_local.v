// A task that waits takes `ref` arguments (IEEE 1800-2017 13.5.2) that are variables of
// the calling automatic subroutine: the task sees and changes the caller's queue and
// variable, before and after its waits.
class Map;
  task automatic perform(ref int q[$], ref int n);
    foreach (q[i]) begin
      #1;
      q[i] = q[i] * 10;
      n++;
    end
    q.push_back(n);
  endtask
  task automatic access(output int sum, output int cnt);
    int q[$];
    int n = 0;
    q.push_back(1);
    q.push_back(2);
    q.push_back(3);
    perform(q, n);
    sum = 0;
    foreach (q[i]) sum += q[i];
    cnt = n;
  endtask
endclass
module t;
  Map m = new;
  int sum, cnt;
  initial begin
    m.access(sum, cnt);
    if (sum == 63 && cnt == 3 && $time == 3) $display("PASSED");
    else $display("FAILED: %0d %0d %0t", sum, cnt, $time);
  end
endmodule
