// Bounded queues ([$:N] holds N + 1 elements) and the run-time warnings of queue
// operations outside the queue.
module t;
  int q[$:2];
  int u[$];
  logic [7:0] lq[$];
  real rq[$:1];
  int i;
  initial begin
    q.push_back(1);
    q.push_back(2);
    q.push_back(3);
    q.push_back(4);
    q.push_front(0);
    $display("%0d %0d %0d size %0d", q[0], q[1], q[2], q.size());
    q.insert(1, 9);
    $display("%0d %0d %0d", q[0], q[1], q[2]);
    q[3] = 5;
    u = '{7, 6, 5, 4, 3};
    q = u;
    $display("%0d %0d %0d size %0d", q[0], q[1], q[2], q.size());
    i = -1;
    lq[i] = 8'h12;
    lq[3] = 8'h34;
    lq.insert(2, 8'h56);
    lq.delete(0);
    i = lq.pop_back();
    rq.push_back(1.5);
    rq.push_back(2.5);
    rq.push_front(0.25);
    $display("%0.2f %0.2f size %0d", rq[0], rq[1], rq.size());
  end
endmodule
